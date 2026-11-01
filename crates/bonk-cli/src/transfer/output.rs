//! Ce que ces commandes écrivent : événements JSON, progression, lignes de résultat.

use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use bonk_core::analysis::{ImageKind, ImageReport};
use bonk_core::write::{Control, Phase, Progress};
use bonk_hal::format_size;
use schemars::JsonSchema;
use serde::Serialize;

use super::args::*;
use crate::errors::{Code, report};
use bonk_ops::job::{self, JobError, RepairDone, VerifyDone, WriteDone};
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

pub(super) fn error_event(error: &JobError, out: &mut dyn Write) {
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
pub(super) fn progress_mode(mode: ProgressMode, quiet: bool) -> ProgressMode {
    match mode {
        ProgressMode::Auto if quiet => ProgressMode::None,
        other => other,
    }
}

/// Rappel de progression selon le mode choisi.
pub(super) fn sink<'a>(
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

pub(super) fn end_progress_line(mode: ProgressMode, err: &mut dyn Write) {
    if mode == ProgressMode::Auto && io::stderr().is_terminal() {
        let _ = writeln!(err);
    }
}

pub(super) fn print_json(
    out: &mut dyn Write,
    value: &impl Serialize,
    compact: bool,
) -> io::Result<()> {
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

pub(super) fn image_line(report: &ImageReport) -> String {
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

pub(super) fn seconds(ms: u64) -> String {
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
