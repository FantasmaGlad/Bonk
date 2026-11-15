use bonk_core::format::FormatError;
use bonk_core::format::exfat_read::FileEntry;
use bonk_core::vault::{self, VaultPlan};
use bonk_hal::{Elevation, FilterOptions};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;
use crate::job::target::open_read_only;
use crate::job::translate::support_lost;
use crate::job::types::{Deps, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};

pub struct VaultListJob {
    pub target: Target,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub elevation: Elevation,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultListDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub layout: VaultPlan,
    /// Sector size of the partition positions.
    pub block_size: u32,
    /// ISO files of the data partition, sorted by path.
    pub images: Vec<FileEntry>,
}

pub(super) fn read_error(error: FormatError) -> JobError {
    match &error {
        FormatError::Device(device) => {
            support_lost(device).unwrap_or_else(|| fail(Code::RawDiskUnavailable, error))
        }
        FormatError::Invalid(_) => fail(Code::NotAVault, error),
        _ => fail(Code::FormatFailed, error),
    }
}

pub fn vault_list(job: &VaultListJob, deps: &Deps<'_>) -> Result<VaultListDone, JobError> {
    let (device, info) =
        open_read_only(&job.target, job.options, job.elevation, job.trace_io, deps)?;
    let layout = vault::detect(&*device)
        .map_err(read_error)?
        .ok_or_else(|| {
            fail(
                Code::NotAVault,
                "no Bonk vault on this disk: prepare it with `bonk vault init`",
            )
        })?;
    let images = vault::images(&*device, &layout).map_err(read_error)?;
    Ok(VaultListDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        target: info,
        layout,
        block_size: device.block_size(),
        images,
    })
}
