use bonk_core::format::Ids;
use bonk_core::vault::{self, VaultBoot};
use bonk_hal::{Access, Elevation, FilterOptions};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;
use crate::job::target::fresh_seed;
use crate::job::types::{Deps, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};
use crate::job::verify::drop_cache_before_verify;

use super::*;

pub struct VaultUpdateJob {
    pub target: Target,
    pub boot: VaultBoot,
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
    /// GRUB for BIOS machines in the MBR and the sectors before the first partition.
    pub bios: bool,
    /// The boot partition already held these exact files, and the MBR this GRUB for BIOS.
    pub up_to_date: bool,
    /// Written now.
    pub updated: bool,
    /// Boot partition and MBR read back after writing.
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
    let block_size = device.block_size();
    let done = |event, updated, verified, target| VaultUpdateDone {
        schema_version: SCHEMA_VERSION,
        event,
        dry_run: job.dry_run,
        target,
        grub: job.grub.clone(),
        bios: job.boot.bios_on(block_size).is_some(),
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
