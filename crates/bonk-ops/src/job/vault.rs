use std::collections::BTreeMap;
use std::path::Path;

use std::time::Instant;

use bonk_core::BlockDevice;
use bonk_core::format::exfat_read::FileEntry;
use bonk_core::format::table::TableKind;
use bonk_core::format::{FormatError, Ids};
use bonk_core::vault::{self, BootFile, VaultPlan};
use bonk_core::write::{Control, Phase, Progress};
use bonk_hal::{Access, Elevation, FilterOptions, TargetIdentity};
use schemars::JsonSchema;
use serde::Serialize;

use super::extract::Advance;
use super::format::{fresh_seed, wait_for_partitions};
use super::target::{OpenRequest, choose, open_read_only, open_target, reread_table};
use super::translate::guarded;
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
    /// SHA-256 of each EFI image written, by processor (`x86_64`, `aarch64`).
    pub efi: BTreeMap<String, String>,
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

/// Adds an ISO to a vault: refused before the drive is touched when the vault menu could not start it.
pub struct VaultAddJob {
    pub target: Target,
    pub source: std::path::PathBuf,
    /// Name on the vault; by default the source's own. Android reads the chosen file through a descriptor
    /// path that carries no name.
    pub name: Option<String>,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub yes: bool,
    pub elevation: Elevation,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultAddDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    /// Name at the root of the data partition.
    pub name: String,
    pub size_bytes: u64,
    pub family: vault::IsoFamily,
    /// The processor it starts on; the vault menu lists the others after this machine's.
    pub arch: vault::IsoArch,
    pub sha256: String,
    /// Read back from the drive after the cache was dropped.
    pub verified: bool,
}

pub struct VaultRemoveJob {
    pub target: Target,
    pub name: String,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub yes: bool,
    pub elevation: Elevation,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultRemoveDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub name: String,
    pub freed_bytes: u64,
}

fn edit_error(error: FormatError) -> JobError {
    match error {
        FormatError::Device(device) => {
            support_lost(&device).unwrap_or_else(|| fail(Code::WriteFailed, device))
        }
        FormatError::Name(what) => fail(Code::VaultEntry, what),
        FormatError::TooLarge(what) => fail(Code::VaultNoRoom, what),
        FormatError::Source(err) if err.kind() == std::io::ErrorKind::Interrupted => fail(
            Code::Cancelled,
            "cancelled during the copy: the vault keeps the space, `bonk vault remove` or a file system check frees it",
        ),
        FormatError::Source(err) => fail(Code::SourceUnreadable, err),
        FormatError::Invalid(_) => fail(Code::NotAVault, error),
        other => fail(Code::WriteFailed, other),
    }
}

/// The ISO's name, family and processor, checked as the vault menu will check them at boot.
fn vault_source(
    path: &Path,
    name: Option<&str>,
) -> Result<(String, u64, vault::IsoKind), JobError> {
    let name = name
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .ok_or_else(|| {
            fail(
                Code::IsoNotForVault,
                format!("{}: no file name", path.display()),
            )
        })?
        .to_owned();
    // The menu lists `*.iso` and `*.ISO` only.
    if !name.to_ascii_lowercase().ends_with(".iso") {
        return Err(fail(
            Code::IsoNotForVault,
            format!("{name}: the vault menu only lists files ending in .iso"),
        ));
    }
    if name.contains(' ') {
        return Err(fail(
            Code::IsoNotForVault,
            format!("{name}: a space in the name; rename it without spaces"),
        ));
    }
    let file = std::fs::File::open(path)
        .map_err(|e| fail(Code::SourceUnreadable, format!("{}: {e}", path.display())))?;
    let size = file
        .metadata()
        .map_err(|e| fail(Code::SourceUnreadable, format!("{}: {e}", path.display())))?
        .len();
    let mut iso = bonk_core::iso9660::Iso::open(std::io::BufReader::new(file))
        .map_err(|e| fail(Code::IsoNotForVault, format!("{name}: {e}")))?;
    let kind = vault::iso_kind(&mut iso)
        .map_err(|e| fail(Code::IsoNotForVault, format!("{name}: {e}")))?
        .ok_or_else(|| {
            fail(
                Code::IsoNotForVault,
                format!("{name}: not Debian live, Ubuntu, Arch Linux or Fedora live"),
            )
        })?;
    Ok((name, size, kind))
}

fn open_vault(
    target: &Target,
    access: Access,
    options: FilterOptions,
    elevation: Elevation,
    trace_io: Option<TraceIo>,
    deps: &Deps<'_>,
) -> Result<(Box<dyn BlockDevice>, TargetInfo, VaultPlan), JobError> {
    let chosen = choose(target, options, deps)?;
    let (device, info, _) = open_target(
        chosen,
        target,
        &OpenRequest {
            options,
            access,
            elevation,
            size_hint: None,
            trace_io,
        },
        deps,
    )?;
    let plan = vault::detect(&*device)
        .map_err(read_error)?
        .ok_or_else(|| {
            fail(
                Code::NotAVault,
                "no Bonk vault on this disk: prepare it with `bonk vault init`",
            )
        })?;
    Ok((device, info, plan))
}

fn confirmed(yes: bool, what: &str) -> Result<(), JobError> {
    if yes {
        Ok(())
    } else {
        Err(fail(
            Code::ConfirmationRequired,
            format!("{what} without confirmation: add --yes"),
        ))
    }
}

/// Replaces the boot partition when it differs from what the catalog gives today; the ISO images stay.
pub struct VaultUpdateJob {
    pub target: Target,
    /// GRUB images, menu and the files it uses, as `vault init` would write them now.
    pub boot: Vec<BootFile>,
    pub grub: Option<GrubInfo>,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub elevation: Elevation,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultUpdateDone {
    pub schema_version: u32,
    /// `plan` or `result`.
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub grub: Option<GrubInfo>,
    /// The boot partition already held these exact files.
    pub up_to_date: bool,
    /// Written now.
    pub updated: bool,
    /// Boot partition read back after writing.
    pub verified: bool,
}

/// # Errors
/// Not a vault, missing confirmation, write or read-back failure.
pub fn vault_update(job: &VaultUpdateJob, deps: &Deps<'_>) -> Result<VaultUpdateDone, JobError> {
    let access = if job.dry_run {
        Access::ReadOnly
    } else {
        Access::ReadWrite
    };
    let (mut device, info, plan) = open_vault(
        &job.target,
        access,
        job.options,
        job.elevation,
        job.trace_io,
        deps,
    )?;
    let up_to_date = vault::boot_matches(&*device, &plan, &job.boot).map_err(read_error)?;
    let done = |event, updated, verified, target| VaultUpdateDone {
        schema_version: SCHEMA_VERSION,
        event,
        dry_run: job.dry_run,
        target,
        grub: job.grub.clone(),
        up_to_date,
        updated,
        verified,
    };
    if up_to_date || job.dry_run {
        return Ok(done(
            if job.dry_run { "plan" } else { "result" },
            false,
            false,
            info,
        ));
    }
    confirmed(job.yes, "vault update")?;
    if (deps.io.cancelled)() {
        return Err(fail(Code::Cancelled, "cancelled before updating the vault"));
    }
    let ids = Ids::from_seed(&fresh_seed(&info.path));
    log::info!(target: "bonk::write", target_path = info.path.as_str(); "vault update starting");
    vault::write_boot(&mut *device, &plan, &ids, &job.boot).map_err(init_error)?;
    device.flush().map_err(|e| fail(Code::FormatFailed, e))?;
    drop_cache_before_verify(&mut *device);
    vault::verify_boot(&*device, &plan, &ids, &job.boot)
        .map_err(|e| fail(Code::VerifyMismatch, e))?;
    log::info!(target: "bonk::write", "vault update finished");
    Ok(done("result", true, true, info))
}

/// # Errors
/// Coded refusal before any write; then write, room or read-back failure.
pub fn vault_add(
    job: &VaultAddJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VaultAddDone, JobError> {
    let (name, size, kind) = vault_source(&job.source, job.name.as_deref())?;
    confirmed(job.yes, "vault add")?;
    let (mut device, info, plan) = open_vault(
        &job.target,
        Access::ReadWrite,
        job.options,
        job.elevation,
        job.trace_io,
        deps,
    )?;
    let mut file = std::fs::File::open(&job.source).map_err(|e| {
        fail(
            Code::SourceUnreadable,
            format!("{}: {e}", job.source.display()),
        )
    })?;
    let mut report = guarded(deps, progress);
    let mut advance = Advance {
        phase: Phase::Write,
        done: 0,
        total: size,
        started: Instant::now(),
        report: &mut report,
    };
    log::info!(target: "bonk::write", name = name.as_str(), bytes = size; "vault add starting");
    let added = vault::add_image(
        &mut *device,
        &plan,
        &name,
        size,
        kind.family,
        &mut file,
        &mut |n| advance.add(n),
    )
    .map_err(edit_error)?;
    device.flush().map_err(|e| fail(Code::WriteFailed, e))?;
    drop_cache_before_verify(&mut *device);
    let mut advance = Advance {
        phase: Phase::Verify,
        done: 0,
        total: size,
        started: Instant::now(),
        report: &mut report,
    };
    vault::verify_image(&*device, &plan, &name, &added.sha256, &mut |n| {
        advance.add(n)
    })
    .map_err(|e| fail(Code::VerifyMismatch, e))?;
    log::info!(target: "bonk::write", name = name.as_str(); "vault add finished");
    Ok(VaultAddDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        target: info,
        name,
        size_bytes: size,
        family: kind.family,
        arch: kind.arch,
        sha256: added.sha256,
        verified: true,
    })
}

/// # Errors
/// Missing confirmation, no such ISO, or write failure.
pub fn vault_remove(job: &VaultRemoveJob, deps: &Deps<'_>) -> Result<VaultRemoveDone, JobError> {
    confirmed(job.yes, "vault remove")?;
    let (mut device, info, plan) = open_vault(
        &job.target,
        Access::ReadWrite,
        job.options,
        job.elevation,
        job.trace_io,
        deps,
    )?;
    let freed = vault::remove_image(&mut *device, &plan, &job.name).map_err(edit_error)?;
    device.flush().map_err(|e| fail(Code::WriteFailed, e))?;
    log::info!(target: "bonk::write", name = job.name.as_str(); "vault remove finished");
    Ok(VaultRemoveDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        target: info,
        name: job.name.clone(),
        freed_bytes: freed,
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
