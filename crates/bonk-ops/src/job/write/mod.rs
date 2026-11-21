use std::path::PathBuf;

use bonk_core::analysis::ImageReport;
use bonk_core::codec::Format;
use bonk_core::format::table::TableKind;
use bonk_core::hybrid::{MBR_LEN, hybrid_mbr};
use bonk_core::read_at::ReadAt;
use bonk_core::write::{Control, Progress};
use bonk_hal::{Elevation, FilterOptions};

use super::source::ImageFile;
use super::target::choose;
use super::types::{Deps, JobError, SourceInfo, Target, TraceIo, WriteMode, WriteOutcome, fail};
use crate::errors::Code;

mod checks;
mod plan;
mod raw;
#[cfg(test)]
mod tests;
mod windows;

pub(in crate::job) use checks::*;
pub(in crate::job) use plan::*;
pub(in crate::job) use raw::*;
pub(in crate::job) use windows::*;

pub struct WriteJob {
    pub target: Target,
    pub source: PathBuf,
    pub options: FilterOptions,
    pub verify: bool,
    pub repair_gpt: bool,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub expected_sha256: Option<String>,
    pub mode: WriteMode,
    /// Windows drive table; no effect on a raw write, whose table comes from the image.
    pub table: TableKind,
    pub elevation: Elevation,
    /// Stands for `yes` and allows an internal disk the filter options already admit. No question: a command
    /// must run with nobody at the screen.
    pub force: bool,
}

/// # Errors
/// `InvalidArgument` unless 64 hexadecimal characters.
pub fn normalize_sha256(text: &str) -> Result<String, JobError> {
    let hex = text.trim().to_ascii_lowercase();
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(hex)
    } else {
        Err(fail(
            Code::InvalidArgument,
            format!("invalid SHA-256 hash: '{hex}' (64 hexadecimal characters)"),
        ))
    }
}

/// What would refuse the write whatever the image: checked before a long download, which must not end on a
/// refusal already known. Touches nothing.
///
/// # Errors
/// Target refused, or confirmation missing.
pub fn precheck(job: &WriteJob, deps: &Deps<'_>) -> Result<(), JobError> {
    let chosen = choose(&job.target, job.options, deps)?;
    if job.dry_run {
        return Ok(());
    }
    confirm(job, &chosen)
}

pub fn write(
    job: &WriteJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteOutcome, JobError> {
    refuse_source_as_target(job)?;
    let file = ImageFile::open(&job.source)?;
    let (source, source_info) = file.decoded()?;
    let chosen = choose(&job.target, job.options, deps)?;
    let (image, head) = analyze_source(&file, &job.source, source_info.size_bytes)?;
    if let Some(windows) = windows_image(&file, &source_info, &image, job.mode)? {
        return write_windows(
            &windows,
            &file,
            source_info,
            image,
            chosen,
            job,
            deps,
            progress,
        );
    }
    let hybrid_mbr = if source_info.format == Format::Raw {
        hybrid_plan(&file, job.mode)?
    } else {
        hybrid_plan(head.as_slice(), job.mode)?
    };
    judge_image(&image, job.mode)?;
    refuse_if_too_large(
        needed_bytes(&source_info, &image, job.mode),
        chosen.info.size_bytes,
    )?;

    if let Some(expected) = &job.expected_sha256 {
        check_sha256(&file, &job.source, expected, deps, progress)?;
    }
    let sha256_checked = job.expected_sha256.is_some();

    if job.dry_run {
        let steps = planned_steps(job, &chosen);
        return Ok(plan(
            chosen,
            source_info,
            image,
            sha256_checked,
            steps,
            None,
        ));
    }
    confirm(job, &chosen)?;
    if !sha256_checked && source_info.format != Format::Raw {
        rehearse(
            &file,
            &image,
            chosen.info.size_bytes,
            job.mode,
            deps,
            progress,
        )?;
    }

    let checked = Checked {
        source_info,
        image,
        sha256_checked,
        hybrid_mbr,
    };
    write_raw(job, chosen, source, checked, deps, progress)
}

pub(in crate::job) struct Checked {
    pub(in crate::job) source_info: SourceInfo,
    pub(in crate::job) image: ImageReport,
    pub(in crate::job) sha256_checked: bool,
    pub(in crate::job) hybrid_mbr: Option<[u8; MBR_LEN]>,
}

/// Refused before the drive is touched. A raw image is read where its structures lie; a compressed one only
/// through its decompressed head.
fn hybrid_plan(
    source: &(impl ReadAt + ?Sized),
    mode: WriteMode,
) -> Result<Option<[u8; MBR_LEN]>, JobError> {
    if mode != WriteMode::Hybrid {
        return Ok(None);
    }
    hybrid_mbr(source)
        .map(Some)
        .map_err(|reason| fail(Code::ImageNotBootable, format!("--mode hybrid: {reason}")))
}
