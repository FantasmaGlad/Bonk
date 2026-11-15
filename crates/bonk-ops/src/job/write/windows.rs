use bonk_core::analysis::ImageReport;
use bonk_core::codec::Format;
use bonk_core::write::{Control, Progress};

use crate::errors::Code;
use crate::job::extract::{self, WindowsImage};
use crate::job::source::ImageFile;
use crate::job::target::Chosen;
use crate::job::types::{Deps, JobError, SourceInfo, WriteMode, WriteOutcome, fail};

use super::*;

/// `extract` requires one; `auto` looks for one where a raw write would be refused; `raw` never looks.
pub(in crate::job) fn windows_image(
    file: &ImageFile,
    source: &SourceInfo,
    image: &ImageReport,
    mode: WriteMode,
) -> Result<Option<WindowsImage>, JobError> {
    let wanted = match mode {
        WriteMode::Raw => false,
        WriteMode::Extract => true,
        WriteMode::Auto => image.udf && !image.is_writable_as_is(),
    };
    if !wanted || source.format != Format::Raw {
        return if mode == WriteMode::Extract {
            Err(fail(
                Code::ImageNotBootable,
                "--mode extract requires an uncompressed Windows installation ISO",
            ))
        } else {
            Ok(None)
        };
    }
    let found = extract::detect(file)?;
    if found.is_none() && mode == WriteMode::Extract {
        return Err(fail(
            Code::ImageNotBootable,
            "this is not a Windows installation ISO (UDF, sources/install.wim or install.esd, efi/boot/bootx64.efi, bootaa64.efi or bootia32.efi)",
        ));
    }
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::job) fn write_windows(
    windows: &WindowsImage,
    file: &ImageFile,
    source: SourceInfo,
    image: ImageReport,
    chosen: Chosen,
    job: &WriteJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteOutcome, JobError> {
    windows.check_fits(chosen.info.size_bytes, chosen.block_size(), job.table)?;
    if let Some(expected) = &job.expected_sha256 {
        check_sha256(file, &job.source, expected, deps, progress)?;
    }
    let sha256_checked = job.expected_sha256.is_some();
    if job.dry_run {
        let block = chosen.block_size();
        let mut steps = vec!["unmount", "write_table", "write_fat32", "copy_files"];
        if chosen.disk.is_none() {
            steps.remove(0);
        }
        steps.push("verify_files");
        if chosen.disk.is_some() {
            steps.push("reread_table");
        }
        return Ok(plan(
            chosen,
            source,
            image,
            sha256_checked,
            steps,
            Some(windows.describe(job.table, block)),
        ));
    }
    confirm(job, &chosen)?;
    extract::run(windows, file, chosen, job, sha256_checked, deps, progress)
        .map(|done| WriteOutcome::Extracted(Box::new(done)))
}
