use std::path::Path;

use bonk_core::analysis::{self, ImageKind, ImageReport};
use bonk_core::write::{self as engine, Control, DEFAULT_CHUNK, Progress, VerifyError};
use bonk_hal::{Eligibility, FilterOptions, Warning, evaluate};

use crate::errors::Code;
use crate::job::source::ImageFile;
use crate::job::target::Chosen;
use crate::job::translate::{guarded, map_verify_error};
use crate::job::types::{Deps, JobError, SourceInfo, Target, WriteMode, fail};

use super::*;

pub(in crate::job) fn confirm(job: &WriteJob, chosen: &Chosen) -> Result<(), JobError> {
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "write without confirmation: add --yes or --force (or describe the operation with --dry-run)",
        ));
    }
    confirm_internal(chosen, job.options, job.force)
}

/// Decompresses the image once without writing. A decompressed stream only shows its end last: without this
/// pass, data after the stream or an image larger than the target would only be seen after the key is erased.
/// A checked hash makes this pass unnecessary.
pub(in crate::job) fn rehearse(
    image_file: &ImageFile,
    image: &ImageReport,
    available: Option<u64>,
    mode: WriteMode,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(), JobError> {
    let (mut source, info) = image_file.decoded()?;
    let mut report = guarded(deps, progress);
    let (len, _) =
        engine::hash_source_ignoring(&mut source.reader, None, DEFAULT_CHUNK, &[], &mut report)
            .map_err(|e| map_verify_error(e, info.format))?;
    if let Some(available) = available.filter(|available| len > *available) {
        return Err(fail(
            Code::SourceTooLarge,
            format!("the decompressed image is {len} bytes, the target {available} bytes"),
        ));
    }
    check_complete(image, len, mode)
}

/// Before any action: a wrong file must cost neither an unmount nor the key's content.
pub(in crate::job) fn check_sha256(
    image: &ImageFile,
    path: &Path,
    expected: &str,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(), JobError> {
    let expected = normalize_sha256(expected)?;
    let mut report = guarded(deps, progress);
    let actual = engine::hash_stream(
        &mut image.raw(),
        Some(image.size()),
        DEFAULT_CHUNK,
        &mut report,
    )
    .map_err(|e| match e {
        VerifyError::Cancelled => fail(Code::Cancelled, e),
        _ => fail(Code::SourceUnreadable, format!("{}: {e}", path.display())),
    })?;
    if actual == expected {
        Ok(())
    } else {
        Err(fail(
            Code::SourceHashMismatch,
            format!("{}: expected {expected}, computed {actual}", path.display()),
        ))
    }
}

/// Through a separate reader: the main stream must stay intact for the write. The head comes back for the
/// hybrid MBR, read from the same bytes as the analysis.
pub(in crate::job) fn analyze_source(
    image: &ImageFile,
    path: &Path,
    real_len: Option<u64>,
) -> Result<(ImageReport, Vec<u8>), JobError> {
    use std::io::Read;

    let (source, _) = image.decoded()?;
    let mut head = Vec::new();
    source
        .reader
        .take(analysis::HEAD_LEN as u64)
        .read_to_end(&mut head)
        .map_err(|e| {
            fail(
                Code::DecompressionFailed,
                format!("{}: {e}", path.display()),
            )
        })?;
    // Whatever the mode: writing nothing and reporting success would be a lie.
    if head.is_empty() {
        return Err(fail(
            Code::SourceUnreadable,
            format!("{}: empty image", path.display()),
        ));
    }
    Ok((analysis::analyze(&head, real_len), head))
}

/// `Raw` overrides it, and logs it.
pub(in crate::job) fn judge_image(report: &ImageReport, mode: WriteMode) -> Result<(), JobError> {
    let problem = if report.truncated == Some(true) {
        Some((
            Code::ImageTruncated,
            format!(
                "the image declares {} bytes",
                report.declared_bytes.unwrap_or_default()
            ),
        ))
    } else if report.is_writable_as_is() {
        None
    } else if report.kind == ImageKind::PlainIso {
        let hint = if report.udf {
            "; Bonk does not recognize a Windows installation in it"
        } else {
            ""
        };
        Some((
            Code::ImageNotBootable,
            format!("ISO without disk boot: it would not boot from a USB drive{hint}"),
        ))
    } else {
        Some((
            Code::ImageNotBootable,
            "no boot structure or partition table recognized".to_owned(),
        ))
    };
    match (problem, mode) {
        (None, _) => Ok(()),
        // The MBR written after the ISO is what makes it start from a drive.
        (Some(_), WriteMode::Hybrid)
            if report.kind == ImageKind::PlainIso && report.truncated != Some(true) =>
        {
            Ok(())
        }
        (Some((code, message)), WriteMode::Auto | WriteMode::Extract | WriteMode::Hybrid) => {
            Err(fail(code, message))
        }
        (Some((code, message)), WriteMode::Raw) => {
            log::warn!(target: "bonk::write", code = code.id(), reason = message.as_str(); "written anyway: raw mode");
            Ok(())
        }
    }
}

/// When known before reading the whole image: a compressed image only gives what its structure declares,
/// ignored in `Raw` mode, which does not trust the analysis.
pub(in crate::job) fn needed_bytes(
    source: &SourceInfo,
    image: &ImageReport,
    mode: WriteMode,
) -> Option<u64> {
    source
        .size_bytes
        .or(image.declared_bytes.filter(|_| mode != WriteMode::Raw))
}

/// A compressed image only gives its real size once read: the read-back compares with the stream, so a
/// complete stream with cut content goes unseen. Checked afterwards, for lack of better.
pub(in crate::job) fn check_complete(
    image: &ImageReport,
    written: u64,
    mode: WriteMode,
) -> Result<(), JobError> {
    let Some(declared) = image.declared_bytes.filter(|declared| written < *declared) else {
        return Ok(());
    };
    let message = format!(
        "the decompressed image is {written} bytes and declares {declared}: the drive is incomplete"
    );
    match mode {
        WriteMode::Auto | WriteMode::Extract | WriteMode::Hybrid => {
            Err(fail(Code::ImageTruncated, message))
        }
        WriteMode::Raw => {
            log::warn!(target: "bonk::write", reason = message.as_str(); "written anyway: raw mode");
            Ok(())
        }
    }
}

/// An offered internal disk needs `force`, `yes` is not enough: erasing a disk of the machine must not come
/// from typing habit. Nothing has moved yet: last check before the first action.
pub(in crate::job) fn confirm_internal(
    chosen: &Chosen,
    options: FilterOptions,
    force: bool,
) -> Result<(), JobError> {
    let Some(disk) = &chosen.disk else {
        return Ok(());
    };
    let internal = evaluate(disk, options)
        == (Eligibility::Offered {
            warning: Warning::InternalDisk,
        });
    if !internal {
        return Ok(());
    }
    if force {
        log::warn!(target: "bonk::write", path = disk.path.as_str(), model = disk.model.as_str(); "internal disk: all its data will be erased, --force given");
        return Ok(());
    }
    Err(fail(
        Code::InternalConfirmationRequired,
        format!(
            "{} ({}) is an internal disk: all its data would be erased. Add --force to confirm",
            disk.path, disk.model
        ),
    ))
}

/// Refused before any action when the image size is known and exceeds the target.
pub(in crate::job) fn refuse_if_too_large(
    needed: Option<u64>,
    available: Option<u64>,
) -> Result<(), JobError> {
    if let (Some(needed), Some(available)) = (needed, available)
        && needed > available
    {
        return Err(fail(
            Code::SourceTooLarge,
            format!("the image is {needed} bytes, the target {available} bytes"),
        ));
    }
    Ok(())
}

/// Simulating onto the image itself would overwrite it while reading: a compressed image would be destroyed.
pub(in crate::job) fn refuse_source_as_target(job: &WriteJob) -> Result<(), JobError> {
    match &job.target {
        Target::Image { path, .. } if bonk_hal::same_file(path, &job.source) => Err(fail(
            Code::InvalidArgument,
            format!(
                "{} is the image itself: choose another target file",
                path.display()
            ),
        )),
        _ => Ok(()),
    }
}
