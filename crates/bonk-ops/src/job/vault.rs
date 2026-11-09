use std::path::Path;

use bonk_core::BlockDevice;
use bonk_core::format::exfat_read::FileEntry;
use bonk_core::format::table::TableKind;
use bonk_core::format::{FormatError, Ids};
use bonk_core::vault::{self, BootFile, VaultPlan};
use bonk_hal::{Access, Elevation, FilterOptions, TargetIdentity};
use schemars::JsonSchema;
use serde::Serialize;

use super::format::{fresh_seed, wait_for_partitions};
use super::target::{OpenRequest, choose, open_read_only, open_target, reread_table};
use super::translate::support_lost;
use super::types::{Deps, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};
use super::verify::drop_cache_before_verify;
use super::write::confirm_internal;
use crate::errors::Code;

/// The GRUB written to the boot partition, as the signed catalog names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GrubInfo {
    pub version: String,
    pub build: String,
    pub arch: String,
    pub sha256: String,
}

pub struct VaultInitJob {
    pub target: Target,
    /// GRUB images and menu; empty: the boot partition stays empty.
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

fn init_error(error: FormatError) -> JobError {
    if let FormatError::Device(device) = &error
        && let Some(lost) = support_lost(device)
    {
        return lost;
    }
    fail(Code::FormatFailed, error)
}

fn read_error(error: FormatError) -> JobError {
    match &error {
        FormatError::Device(device) => {
            support_lost(device).unwrap_or_else(|| fail(Code::RawDiskUnavailable, error))
        }
        FormatError::Invalid(_) => fail(Code::NotAVault, error),
        _ => fail(Code::FormatFailed, error),
    }
}

fn write_vault(
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

pub fn vault_init(job: &VaultInitJob, deps: &Deps<'_>) -> Result<VaultInitDone, JobError> {
    let chosen = choose(&job.target, job.options, deps)?;
    let size = chosen.info.size_bytes.ok_or_else(|| {
        fail(
            Code::InvalidArgument,
            "target image size unknown: give it with --size",
        )
    })?;
    let planned = vault::plan(size, chosen.block_size(), job.table).map_err(init_error)?;
    let result = |event, dry_run, target, layout, block_size, unmounted, verified| VaultInitDone {
        schema_version: SCHEMA_VERSION,
        event,
        dry_run,
        target,
        layout,
        block_size,
        vault_label: vault::VAULT_LABEL,
        efi_label: vault::EFI_LABEL,
        grub: job.grub.clone(),
        unmounted,
        verified,
    };
    if job.dry_run {
        let block = chosen.block_size();
        return Ok(result(
            "plan",
            true,
            chosen.info,
            planned,
            block,
            chosen.mounts,
            false,
        ));
    }
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "vault init without confirmation: add --yes or --force (or describe the operation with --dry-run)",
        ));
    }
    confirm_internal(&chosen, job.options, job.force)?;

    let seed_name = chosen.info.path.clone();
    let identity = chosen.disk.as_ref().map(TargetIdentity::from);
    let (mut device, info, unmounted) = open_target(
        chosen,
        &job.target,
        &OpenRequest {
            options: job.options,
            access: Access::ReadWrite,
            elevation: job.elevation,
            size_hint: Some(size),
            trace_io: job.trace_io,
        },
        deps,
    )?;
    if (deps.io.cancelled)() {
        return Err(fail(Code::Cancelled, "cancelled before writing the vault"));
    }
    let ids = Ids::from_seed(&fresh_seed(&seed_name));
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
            // Both volumes are written and read back: the system will see them at the next plug.
            log::warn!(target: "bonk::write", "new partitions not visible yet: unplug and replug the disk");
        }
    }
    log::info!(target: "bonk::write", "vault init finished");
    Ok(result(
        "result", false, info, plan, block_size, unmounted, true,
    ))
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use bonk_core::MemoryDevice;

    use super::*;

    /// Cancel is seen between steps: no volume after a cancelled table.
    #[test]
    fn l_annulation_arrete_avant_les_volumes() {
        let mut device = MemoryDevice::new(256 << 20, 512).unwrap();
        let result = write_vault(
            &mut device,
            TableKind::Mbr,
            &Ids::from_seed(b"essai"),
            &[],
            &|| true,
        );
        assert_eq!(result.err().map(|e| e.code), Some(Code::Cancelled));
        assert_eq!(vault::detect(&device).unwrap(), None);
    }

    #[test]
    fn un_disque_trop_petit_ne_recoit_rien() {
        let mut device = MemoryDevice::new(100 << 20, 512).unwrap();
        let result = write_vault(
            &mut device,
            TableKind::Mbr,
            &Ids::from_seed(b"essai"),
            &[],
            &|| false,
        );
        assert_eq!(result.err().map(|e| e.code), Some(Code::FormatFailed));
        assert!(device.as_bytes().iter().all(|b| *b == 0));
    }
}
