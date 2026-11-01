//! `bonk write`.

use std::io::{self, Write};

use bonk_core::write::VerifyReport;
use bonk_hal::format_size;

use super::args::*;
use super::output::*;
use crate::fetch;
use bonk_ops::job::{self, Deps, TraceIo, WriteJob, WriteOutcome};
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

pub(super) fn extraction_lines(
    extraction: &job::Extraction,
    out: &mut dyn Write,
) -> io::Result<()> {
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

pub(super) fn cache_warning(report: &VerifyReport, out: &mut dyn Write) -> io::Result<()> {
    if report.cache_dropped {
        return Ok(());
    }
    writeln!(
        out,
        "  Warning: the system cache could not be dropped, the read-back may have read memory and not the drive"
    )
}
