use bonk_core::analysis::ImageReport;
use bonk_core::write::{Control, Progress};

use crate::job::extract::{self, WindowsImage};
use crate::job::source::ImageFile;
use crate::job::target::Chosen;
use crate::job::types::{Deps, JobError, SourceInfo, WriteOutcome};

use super::*;

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
            Vec::new(),
        ));
    }
    confirm(job, &chosen)?;
    extract::run(windows, file, chosen, job, sha256_checked, deps, progress)
        .map(|done| WriteOutcome::Extracted(Box::new(done)))
}
