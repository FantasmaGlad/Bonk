//! Commandes `bonk write`, `bonk verify` et `bonk repair-gpt` : arguments, progression, affichage.
//!
//! La logique est dans `bonk_ops::job` ; ce module la présente. Avec `--progress json`, la sortie
//! standard est un flux de lignes JSON (événements `progress` puis un événement `result`), les mêmes
//! événements que la barre de la fenêtre (`repair-gpt` ne rend que le `result`) ; sinon le résultat est du texte ou, avec `--json`, un JSON.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_core::analysis::{ImageKind, ImageReport};
use bonk_core::write::{Control, Phase, Progress, VerifyReport};
use bonk_hal::{Elevation, FilterOptions, format_size};
use clap::{Args, ValueEnum};
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ViewDefaults;
use crate::errors::{Code, report};
use crate::fetch;
use crate::format::TableArg;
use bonk_ops::job::{
    self, Deps, JobError, RepairDone, RepairJob, Target, TraceIo, VerifyDone, VerifyJob, WriteDone,
    WriteJob, WriteMode, WriteOutcome,
};

/// Niveau de `--trace-io` (voir [`TraceIo`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TraceIoArg {
    /// Offset, length, duration and checksum of each access
    Metadata,
    /// With the first 16 bytes: for image files only
    Data,
}

impl From<TraceIoArg> for TraceIo {
    fn from(arg: TraceIoArg) -> Self {
        match arg {
            TraceIoArg::Metadata => Self::Metadata,
            TraceIoArg::Data => Self::Data,
        }
    }
}

/// Valeur de `--elevate` (voir [`Elevation`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ElevateArg {
    /// Ask for system authorization when opening the disk
    Auto,
    /// The caller's rights only
    Never,
}

impl From<ElevateArg> for Elevation {
    fn from(arg: ElevateArg) -> Self {
        match arg {
            ElevateArg::Auto => Self::Auto,
            ElevateArg::Never => Self::Never,
        }
    }
}

/// Mode de `--mode` (voir [`WriteMode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ModeArg {
    /// Extract a Windows ISO; refuse a truncated image or one that would not boot as written
    Auto,
    /// Write the bytes as they are, whatever the analysis says
    Raw,
    /// Copy the files of a Windows installation ISO to a FAT32 partition; auto chooses it by itself
    Extract,
}

impl From<ModeArg> for WriteMode {
    fn from(arg: ModeArg) -> Self {
        match arg {
            ModeArg::Auto => Self::Auto,
            ModeArg::Raw => Self::Raw,
            ModeArg::Extract => Self::Extract,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ProgressMode {
    /// A progress line on standard error if it is a terminal, nothing otherwise
    Auto,
    /// One JSON event per line on standard output
    Json,
    /// No progress
    None,
}

/// Options communes aux opérations sur un disque.
#[derive(Debug, Args)]
pub(crate) struct CommonArgs {
    /// Stable identifier of the disk (see `bonk list`); with --simulate, path of the target image file
    pub(crate) target: String,

    /// The target is an image file (created if missing, with --size), not a disk: replays
    /// the operation without hardware
    #[arg(long)]
    pub(crate) simulate: bool,

    /// Size of the target image created by --simulate (decimal or 0x...); by default that of the source
    /// image when it is raw
    #[arg(long, value_name = "BYTES", value_parser = crate::debug::parse_number)]
    size: Option<u64>,

    /// Allow internal non-system disks (stronger warning)
    #[arg(long)]
    show_internal: bool,

    /// Test mode: allow virtual disks
    #[arg(long)]
    allow_virtual: bool,

    /// Log each block access (offset, duration, checksum); `=data` adds the
    /// first bytes, for an image file only
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "metadata", require_equals = true)]
    pub(crate) trace_io: Option<TraceIoArg>,

    /// Missing rights: auto (ask the system) or never (the caller's rights only)
    #[arg(long, value_enum, default_value_t = ElevateArg::Auto)]
    pub(crate) elevate: ElevateArg,

    /// Progress: auto, json (one event per line) or none
    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    progress: ProgressMode,

    /// Result as JSON
    #[arg(long)]
    json: bool,
}

impl CommonArgs {
    pub(crate) fn target(&self) -> Target {
        if self.simulate {
            Target::Image {
                path: PathBuf::from(&self.target),
                size: self.size,
            }
        } else {
            Target::Disk(self.target.clone())
        }
    }

    fn options(&self, view: &ViewDefaults) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal || view.show_internal,
            allow_virtual: self.allow_virtual || view.allow_virtual,
        }
    }

    /// Pour écrire, seules les options de la commande comptent : un réglage ou une variable oubliés règlent
    /// l'affichage, ils n'autorisent jamais à effacer un disque de plus.
    pub(crate) fn write_options(&self) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
        }
    }
}

/// Write an image to a disk (or an image file with --simulate), then verify it
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct WriteArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Image to write: a file (.img, .iso, .gz, .xz, .zst; the format is recognized by its content) or
    /// an https:// address to download, with its hash
    #[arg(required_unless_present = "image", conflicts_with = "image")]
    source: Option<PathBuf>,

    /// Image of the signed catalog (`bonk catalog list`): its official link and hash are used
    #[arg(long, value_name = "ID", conflicts_with_all = ["sha256", "sha256_file"])]
    image: Option<String>,

    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL", requires = "image")]
    catalog_url: Option<String>,

    /// Confirm the write: without --yes or --force, Bonk does nothing (it never asks a question)
    #[arg(long)]
    yes: bool,

    /// Describe what would be done without writing to the drive; an address is still downloaded
    /// into the cache, since the analysis needs the image
    #[arg(long)]
    dry_run: bool,

    /// Published SHA-256 hash of this file: checked before touching the drive
    #[arg(long, value_name = "HEX", conflicts_with = "sha256_file")]
    sha256: Option<String>,

    /// Published hash file (SHA256SUMS), path or https:// address: the hash is read under the name of
    /// the image
    #[arg(long, value_name = "PATH_OR_URL")]
    sha256_file: Option<String>,

    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,

    /// auto: analyze the image, extract a Windows ISO and refuse what would not boot; raw: write the
    /// bytes as they are; extract: require a Windows ISO
    #[arg(long, value_enum, default_value_t = ModeArg::Auto)]
    mode: ModeArg,

    /// Partition table of a Windows drive; boot stays UEFI in both cases
    #[arg(long, value_enum, default_value_t = TableArg::Gpt)]
    table: TableArg,

    /// Do not read the drive back after writing
    #[arg(long)]
    no_verify: bool,

    /// Do not repair the backup GPT after writing
    #[arg(long)]
    no_repair_gpt: bool,

    /// Confirm like --yes, and allow an internal disk admitted by --show-internal, without a question.
    /// Never a system disk: the safety filter stays in charge
    #[arg(long)]
    force: bool,
}

impl WriteArgs {
    /// Cette commande ouvrirait-elle un vrai disque ? (`--dry-run` et `--simulate` : non)
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate && !self.dry_run
    }
}

/// Compare a disk (or an image) to an image, writing nothing
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct VerifyArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Reference image
    source: PathBuf,
}

impl VerifyArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate
    }
}

/// Repair the GPT of a disk (or an image): backup at the end of the drive, primary restored
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct RepairArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Confirm the repair
    #[arg(long)]
    yes: bool,

    /// Say whether there is anything to repair, writing nothing
    #[arg(long)]
    dry_run: bool,

    /// Confirm like --yes, and allow an internal disk admitted by --show-internal, without a question.
    /// Never a system disk: the safety filter stays in charge
    #[arg(long)]
    force: bool,
}

impl RepairArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate
    }
}

/// Progress event, as it comes out in `--progress json`.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ProgressEvent<'a> {
    schema_version: u32,
    event: &'static str,
    #[serde(flatten)]
    progress: &'a Progress,
}

/// Last event of the `--progress json` stream when the operation fails: a reader of the stream does not have to interpret the standard error text.
#[derive(Serialize, JsonSchema)]
pub(crate) struct ErrorEvent<'a> {
    schema_version: u32,
    event: &'static str,
    code: &'static str,
    message: &'a str,
}

fn error_event(error: &JobError, out: &mut dyn Write) {
    let event = ErrorEvent {
        schema_version: job::SCHEMA_VERSION,
        event: "error",
        code: error.code.id(),
        message: &error.message,
    };
    if let Ok(line) = serde_json::to_string(&event) {
        let _ = writeln!(out, "{line}");
        let _ = out.flush();
    }
}

pub(crate) fn schema_error() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ErrorEvent)).unwrap_or_default()
}

pub(crate) fn schema_write_extract() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(job::ExtractDone)).unwrap_or_default()
}

pub(crate) fn schema_write() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(WriteDone)).unwrap_or_default()
}

pub(crate) fn schema_plan() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(job::Plan)).unwrap_or_default()
}

pub(crate) fn schema_verify() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VerifyDone)).unwrap_or_default()
}

pub(crate) fn schema_repair() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(RepairDone)).unwrap_or_default()
}

pub(crate) fn schema_progress() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ProgressEvent)).unwrap_or_default()
}

/// `--quiet` retire la ligne de progression automatique ; `--progress json`, demandé explicitement, reste.
fn progress_mode(mode: ProgressMode, quiet: bool) -> ProgressMode {
    match mode {
        ProgressMode::Auto if quiet => ProgressMode::None,
        other => other,
    }
}

/// Rappel de progression selon le mode choisi.
fn sink<'a>(
    mode: ProgressMode,
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
) -> impl FnMut(&Progress) -> Control + 'a {
    let tty = io::stderr().is_terminal();
    let mut last = Instant::now() - Duration::from_secs(1);
    move |p| {
        let boundary = matches!(p.phase, Phase::Flush) || p.bytes_done == 0;
        match mode {
            ProgressMode::None => {}
            ProgressMode::Json => {
                // Chaque événement part tel quel : un outil qui lit le flux veut tous les paliers.
                let event = ProgressEvent {
                    schema_version: job::SCHEMA_VERSION,
                    event: "progress",
                    progress: p,
                };
                if let Ok(line) = serde_json::to_string(&event) {
                    let _ = writeln!(out, "{line}");
                    let _ = out.flush();
                }
            }
            ProgressMode::Auto if tty => {
                if boundary || last.elapsed() >= Duration::from_millis(250) {
                    last = Instant::now();
                    let step = match p.phase {
                        Phase::Download => "download",
                        Phase::Authorize => "system authorization",
                        Phase::Source => "hash check",
                        Phase::Write => "write",
                        Phase::Flush => "flushing buffers",
                        Phase::Verify => "verification",
                    };
                    let total = p.bytes_total.map_or_else(String::new, |t| {
                        format!(" / {}", format_size(t, bonk_hal::Lang::En))
                    });
                    let _ = write!(
                        err,
                        "\r{step}: {}{total} ({}/s)   ",
                        format_size(p.bytes_done, bonk_hal::Lang::En),
                        format_size(p.bytes_per_second as u64, bonk_hal::Lang::En)
                    );
                    let _ = err.flush();
                }
            }
            ProgressMode::Auto => {}
        }
        Control::Continue
    }
}

fn end_progress_line(mode: ProgressMode, err: &mut dyn Write) {
    if mode == ProgressMode::Auto && io::stderr().is_terminal() {
        let _ = writeln!(err);
    }
}

fn print_json(out: &mut dyn Write, value: &impl Serialize, compact: bool) -> io::Result<()> {
    if compact {
        writeln!(
            out,
            "{}",
            serde_json::to_string(value).map_err(io::Error::from)?
        )
    } else {
        serde_json::to_writer_pretty(&mut *out, value).map_err(io::Error::from)?;
        writeln!(out)
    }
}

pub(crate) fn target_line(info: &job::TargetInfo) -> String {
    match &info.id {
        Some(id) => format!("disk {id} ({})", info.path),
        None => format!("image {}", info.path),
    }
}

fn image_line(report: &ImageReport) -> String {
    let kind = match report.kind {
        ImageKind::HybridIso => "hybrid ISO",
        ImageKind::BootableDisk => "bootable disk image",
        ImageKind::DataDisk => "image without PC boot (ARM board or data)",
        ImageKind::PlainIso => "ISO without disk boot",
        ImageKind::Unknown => "not recognized",
    };
    let boot: Vec<&str> = [(report.bios, "BIOS"), (report.uefi, "UEFI")]
        .into_iter()
        .filter_map(|(present, name)| present.then_some(name))
        .collect();
    if boot.is_empty() {
        kind.to_owned()
    } else {
        format!("{kind} ({})", boot.join(", "))
    }
}

fn seconds(ms: u64) -> String {
    format!("{}.{:01} s", ms / 1000, ms % 1000 / 100)
}

pub(crate) fn finish<T: Serialize>(
    result: Result<T, JobError>,
    args: &CommonArgs,
    out: &mut dyn Write,
    err: &mut dyn Write,
    text: impl FnOnce(&T, &mut dyn Write) -> io::Result<()>,
) -> u8 {
    end_progress_line(args.progress, err);
    match result {
        Err(e) => {
            if args.progress == ProgressMode::Json {
                error_event(&e, out);
            }
            report(err, e.code, e.message)
        }
        Ok(value) => {
            let written = if args.progress == ProgressMode::Json {
                print_json(out, &value, true)
            } else if args.json {
                print_json(out, &value, false)
            } else {
                text(&value, out)
            };
            match written {
                Ok(()) => 0,
                Err(e) => report(err, Code::OutputFailed, e),
            }
        }
    }
}

pub(crate) fn run_write(
    args: &WriteArgs,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let (source, expected_sha256) = match &args.image {
        Some(id) => {
            match crate::catalog::image_source(
                id,
                args.catalog_url.as_deref(),
                args.cache_dir.as_deref(),
            ) {
                Ok((source, sha256)) => (source, Some(sha256)),
                Err(error) => {
                    return finish::<()>(Err(error), &args.common, out, err, |_, _| Ok(()));
                }
            }
        }
        None => (args.source.clone().unwrap_or_default(), args.sha256.clone()),
    };
    let mut job = WriteJob {
        target: args.common.target(),
        source: source.clone(),
        options: args.common.write_options(),
        verify: !args.no_verify,
        repair_gpt: !args.no_repair_gpt,
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        expected_sha256,
        mode: args.mode.into(),
        table: args.table.into(),
        elevation: args.common.elevate.into(),
        force: args.force,
    };
    log::debug!(target: "bonk::cli", target_kind = if args.common.simulate { "image" } else { "disk" }, dry_run = args.dry_run; "command write");
    let result = {
        let mut progress = sink(
            progress_mode(args.common.progress, quiet),
            &mut *out,
            &mut *err,
        );
        let request = fetch::Request {
            source: &source,
            sha256_file: args.sha256_file.as_deref(),
            cache_dir: args.cache_dir.as_deref(),
        };
        fetch::write(&request, &mut job, deps, &mut progress)
    };
    let code = finish(
        result,
        &args.common,
        out,
        err,
        |outcome, out| match outcome {
            WriteOutcome::Plan(plan) => {
                writeln!(out, "Plan (nothing is written):")?;
                writeln!(out, "  Target: {}", target_line(&plan.target))?;
                writeln!(
                    out,
                    "  Source: {} ({}{})",
                    plan.source.path,
                    plan.source.format,
                    plan.source.size_bytes.map_or_else(String::new, |s| format!(
                        ", {}",
                        format_size(s, bonk_hal::Lang::En)
                    ))
                )?;
                writeln!(out, "  Image: {}", image_line(&plan.image))?;
                if plan.sha256_checked {
                    writeln!(out, "  File hash: matches")?;
                }
                if !plan.unmount.is_empty() {
                    writeln!(out, "  Unmount: {}", plan.unmount.join(", "))?;
                }
                if let Some(extraction) = &plan.extraction {
                    extraction_lines(extraction, out)?;
                }
                writeln!(out, "  Steps: {}", plan.steps.join(", "))
            }
            WriteOutcome::Extracted(done) => {
                writeln!(
                    out,
                    "Windows drive written: {} in {} ({}/s)",
                    format_size(done.write.bytes, bonk_hal::Lang::En),
                    seconds(done.write.elapsed_ms),
                    format_size(done.write.bytes_per_second as u64, bonk_hal::Lang::En)
                )?;
                writeln!(out, "  Target: {}", target_line(&done.target))?;
                extraction_lines(&done.extraction, out)?;
                writeln!(
                    out,
                    "  Verification: passed ({} files read back, {} bytes)",
                    done.verify.files, done.verify.bytes
                )?;
                if !done.verify.cache_dropped {
                    writeln!(
                        out,
                        "  Warning: the system cache could not be dropped, the read-back may have read memory and not the drive"
                    )?;
                }
                Ok(())
            }
            WriteOutcome::Done(done) => {
                writeln!(
                    out,
                    "Write finished: {} in {} ({}/s)",
                    format_size(done.write.bytes, bonk_hal::Lang::En),
                    seconds(done.write.elapsed_ms),
                    format_size(done.write.bytes_per_second as u64, bonk_hal::Lang::En)
                )?;
                writeln!(out, "  Target: {}", target_line(&done.target))?;
                writeln!(
                    out,
                    "  Source: {} ({})",
                    done.source.path, done.source.format
                )?;
                writeln!(out, "  Image: {}", image_line(&done.image))?;
                if done.sha256_checked {
                    writeln!(out, "  File hash: matches")?;
                }
                if !done.unmounted.is_empty() {
                    writeln!(out, "  Unmounted: {}", done.unmounted.join(", "))?;
                }
                writeln!(out, "  SHA-256: {}", done.write.sha256)?;
                match &done.verify {
                    Some(v) => {
                        writeln!(out, "  Verification: passed ({} bytes read back)", v.bytes)?;
                        cache_warning(v, out)?;
                    }
                    None => writeln!(out, "  Verification: not requested")?,
                }
                match &done.gpt_repair {
                    Some(r) => writeln!(out, "  GPT: repaired ({})", r.actions.join(", ")),
                    None if args.no_repair_gpt => writeln!(out, "  GPT: check not requested"),
                    None => writeln!(out, "  GPT: nothing to repair"),
                }
            }
        },
    );
    // Le téléchargement a pu faire grossir le cache : le dire, seulement à qui lit du texte.
    let text = !args.common.json && args.common.progress != ProgressMode::Json;
    let downloaded = source.to_str().is_some_and(bonk_catalog::is_url);
    if code == 0
        && text
        && downloaded
        && let Some(bytes) = crate::cache::directory(args.cache_dir.as_deref())
            .and_then(|dir| crate::cache::total_bytes(&dir).ok())
    {
        let _ = writeln!(
            out,
            "  Download cache: {} (bonk cache clean to empty it)",
            format_size(bytes, bonk_hal::Lang::En)
        );
    }
    code
}

pub(crate) fn run_verify(
    args: &VerifyArgs,
    view: &ViewDefaults,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = VerifyJob {
        target: args.common.target(),
        source: args.source.clone(),
        options: args.common.options(view),
        trace_io: args.common.trace_io.map(TraceIo::from),
        elevation: args.common.elevate.into(),
    };
    let result = {
        let mut progress = sink(
            progress_mode(args.common.progress, quiet),
            &mut *out,
            &mut *err,
        );
        job::verify(&job, deps, &mut progress)
    };
    finish(result, &args.common, out, err, |done, out| {
        writeln!(
            out,
            "Verification passed: {} identical to the image ({})",
            format_size(done.verify.bytes, bonk_hal::Lang::En),
            target_line(&done.target)
        )?;
        if !done.ignored_ranges.is_empty() {
            let octets: u64 = done.ignored_ranges.iter().map(|r| r.length).sum();
            writeln!(
                out,
                "  Excluded from the comparison: {octets} bytes of GPT fields its repair rewrites"
            )?;
        }
        cache_warning(&done.verify, out)?;
        writeln!(out, "  SHA-256: {}", done.verify.sha256)
    })
}

fn extraction_lines(extraction: &job::Extraction, out: &mut dyn Write) -> io::Result<()> {
    writeln!(
        out,
        "  Mode: extraction, {} files, FAT32 on {}, UEFI boot",
        extraction.files,
        extraction.table.name()
    )?;
    if extraction.wim_parts > 0 {
        writeln!(
            out,
            "  install.wim split into {} .swm parts, without recompression",
            extraction.wim_parts
        )?;
    }
    writeln!(out, "  Secure Boot: {}", extraction.secure_boot)
}

fn cache_warning(report: &VerifyReport, out: &mut dyn Write) -> io::Result<()> {
    if report.cache_dropped {
        return Ok(());
    }
    writeln!(
        out,
        "  Warning: the system cache could not be dropped, the read-back may have read memory and not the drive"
    )
}

pub(crate) fn run_repair(
    args: &RepairArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = RepairJob {
        target: args.common.target(),
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        elevation: args.common.elevate.into(),
        force: args.force,
    };
    let result = job::repair(&job, deps);
    finish(result, &args.common, out, err, |done, out| {
        writeln!(
            out,
            "{}: {}",
            target_line(&done.target),
            if done.needs_repair {
                "GPT needs repair"
            } else {
                "GPT in order"
            }
        )?;
        if !done.findings_before.is_empty() {
            writeln!(
                out,
                "  Findings before: {}",
                done.findings_before.join(", ")
            )?;
        }
        if let Some(r) = &done.repair {
            writeln!(out, "  Repaired: {}", r.actions.join(", "))?;
        }
        if let Some(after) = &done.findings_after {
            writeln!(
                out,
                "  Findings after: {}",
                if after.is_empty() {
                    "none".to_owned()
                } else {
                    after.join(", ")
                }
            )?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_retire_seulement_la_progression_automatique() {
        assert!(matches!(
            progress_mode(ProgressMode::Auto, true),
            ProgressMode::None
        ));
        assert!(matches!(
            progress_mode(ProgressMode::Auto, false),
            ProgressMode::Auto
        ));
        assert!(matches!(
            progress_mode(ProgressMode::Json, true),
            ProgressMode::Json
        ));
    }
}
