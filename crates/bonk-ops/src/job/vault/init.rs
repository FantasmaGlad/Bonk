use std::path::Path;

use bonk_core::BlockDevice;
use bonk_core::format::table::TableKind;
use bonk_core::format::{FormatError, Ids};
use bonk_core::vault::{self, BootFile, VaultPlan};
use bonk_hal::{Elevation, FilterOptions};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;
use crate::job::format::wait_for_partitions;
use crate::job::target::{Chosen, OpenRequest, choose, reread_table};
use crate::job::target::{OpenedForWriting, open_for_writing};
use crate::job::translate::support_lost;
use crate::job::types::{Deps, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};
use crate::job::verify::drop_cache_before_verify;
use crate::job::write::confirm_internal;

use super::*;

pub struct VaultInitJob {
    pub target: Target,
    pub boot: Vec<BootFile>,
    pub grub: Option<GrubInfo>,
    pub table: TableKind,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub force: bool,
    pub elevation: Elevation,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultInitDone {
    pub schema_version: u32,
    /// `plan` or `result`.
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub layout: VaultPlan,
    /// Sector size of the partition positions.
    pub block_size: u32,
    pub vault_label: &'static str,
    pub efi_label: &'static str,
    /// Absent: the boot partition was left empty.
    pub grub: Option<GrubInfo>,
    /// Mount points unmounted, or that would be.
    pub unmounted: Vec<String>,
    /// Table and both volumes read back after writing.
    pub verified: bool,
}

pub(super) fn init_error(error: FormatError) -> JobError {
    if let FormatError::Device(device) = &error
        && let Some(lost) = support_lost(device)
    {
        return lost;
    }
    fail(Code::FormatFailed, error)
}

pub(super) fn write_vault(
    device: &mut dyn BlockDevice,
    table: TableKind,
    ids: &Ids,
    boot: &[BootFile],
    cancelled: &dyn Fn() -> bool,
) -> Result<VaultPlan, JobError> {
    // On the real drive: the plan before opening counted the sectors the system reported.
    let plan = vault::plan(device.size(), device.block_size(), table).map_err(init_error)?;
    vault::write_table(device, &plan, ids).map_err(init_error)?;
    if cancelled() {
        return Err(fail(
            Code::Cancelled,
            "cancelled after writing the table: the drive must be prepared again",
        ));
    }
    vault::write_volumes(device, &plan, ids, boot).map_err(init_error)?;
    device.flush().map_err(|e| fail(Code::FormatFailed, e))?;
    drop_cache_before_verify(device);
    vault::verify(device, &plan, ids, boot).map_err(init_error)?;
    Ok(plan)
}

pub(super) fn init_done(
    job: &VaultInitJob,
    target: TargetInfo,
    layout: VaultPlan,
    block_size: u32,
    unmounted: Vec<String>,
) -> VaultInitDone {
    VaultInitDone {
        schema_version: SCHEMA_VERSION,
        event: if job.dry_run { "plan" } else { "result" },
        dry_run: job.dry_run,
        target,
        layout,
        block_size,
        vault_label: vault::VAULT_LABEL,
        efi_label: vault::EFI_LABEL,
        grub: job.grub.clone(),
        unmounted,
        verified: !job.dry_run,
    }
}

pub fn vault_init(job: &VaultInitJob, deps: &Deps<'_>) -> Result<VaultInitDone, JobError> {
    let chosen = choose(&job.target, job.options, deps)?;
    let size = chosen.info.size_bytes.ok_or_else(|| {
        fail(
            Code::InvalidArgument,
            "target image size unknown: give it with --size",
        )
    })?;
    let planned = vault::plan(size, chosen.block_size(), job.table).map_err(init_error)?;
    if job.dry_run {
        let block = chosen.block_size();
        return Ok(init_done(job, chosen.info, planned, block, chosen.mounts));
    }
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "vault init without confirmation: add --yes or --force (or describe the operation with --dry-run)",
        ));
    }
    confirm_internal(&chosen, job.options, job.force)?;
    init_target(job, chosen, size, deps)
}

pub(super) fn init_target(
    job: &VaultInitJob,
    chosen: Chosen,
    size: u64,
    deps: &Deps<'_>,
) -> Result<VaultInitDone, JobError> {
    let request = OpenRequest::read_write(job.options, job.elevation, job.trace_io, Some(size));
    let OpenedForWriting {
        mut device,
        info,
        unmounted,
        ids,
        identity,
    } = open_for_writing(chosen, &job.target, &request, deps, "writing the vault")?;
    log::info!(
        target: "bonk::write",
        target_path = info.path.as_str(), table = job.table.name();
        "vault init starting"
    );
    let plan = write_vault(
        &mut *device,
        job.table,
        &ids,
        &job.boot,
        &*deps.io.cancelled,
    )?;
    let block_size = device.block_size();
    let block = u64::from(block_size);
    drop(device);
    if let Some(identity) = &identity {
        reread_table(Path::new(&info.path), job.elevation, deps);
        let sizes = [plan.vault.sectors() * block, plan.efi.sectors() * block];
        if wait_for_partitions(&identity.id, &sizes, deps)?.is_none() {
            log::warn!(target: "bonk::write", "new partitions not visible yet: unplug and replug the disk");
        }
    }
    log::info!(target: "bonk::write", "vault init finished");
    Ok(init_done(job, info, plan, block_size, unmounted))
}
