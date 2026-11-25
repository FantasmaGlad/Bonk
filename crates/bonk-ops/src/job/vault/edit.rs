use std::path::Path;

use bonk_core::format::FormatError;
use bonk_core::vault;
use bonk_core::write::{Control, Phase, Progress};
use bonk_hal::{Access, Elevation, FilterOptions};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;
use crate::job::advance::Advance;
use crate::job::translate::guarded;
use crate::job::translate::support_lost;
use crate::job::types::{Deps, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};
use crate::job::verify::drop_cache_before_verify;

use super::*;

pub struct VaultAddJob {
    pub target: Target,
    pub source: std::path::PathBuf,
    /// Android reads the chosen file through a descriptor path that carries no name.
    pub name: Option<String>,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub yes: bool,
    pub elevation: Elevation,
    /// Families checked before the drive is opened; the vault's own menu has the last word.
    pub families: Vec<vault::VaultFamily>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct VaultAddDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    /// Name at the root of the data partition.
    pub name: String,
    pub size_bytes: u64,
    /// Identifier of the vault family the menu starts it as: `debian_live`, `ubuntu`, `arch`.
    pub family: String,
    pub family_name: String,
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

pub(super) fn edit_error(error: FormatError) -> JobError {
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

/// The menu lists `*.iso` and `*.ISO` only, and a live kernel line splits on spaces: they become `_`.
pub(super) fn vault_name(path: &Path, name: Option<&str>) -> Result<String, JobError> {
    let given = name
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .ok_or_else(|| {
            fail(
                Code::IsoNotForVault,
                format!("{}: no file name", path.display()),
            )
        })?;
    if !given.to_ascii_lowercase().ends_with(".iso") {
        return Err(fail(
            Code::IsoNotForVault,
            format!("{given}: the vault menu only lists files ending in .iso"),
        ));
    }
    let name = given.replace(' ', "_");
    if name != given {
        log::info!(target: "bonk::vault", given = given, kept = name.as_str(); "spaces written as _");
    }
    Ok(name)
}

pub(super) fn vault_source(
    path: &Path,
    name: Option<&str>,
    families: &[vault::VaultFamily],
) -> Result<(String, u64, vault::IsoKind), JobError> {
    let name = vault_name(path, name)?;
    let file = std::fs::File::open(path)
        .map_err(|e| fail(Code::SourceUnreadable, format!("{}: {e}", path.display())))?;
    let size = file
        .metadata()
        .map_err(|e| fail(Code::SourceUnreadable, format!("{}: {e}", path.display())))?
        .len();
    let mut iso = bonk_core::iso9660::Iso::open(std::io::BufReader::new(file))
        .map_err(|e| fail(Code::IsoNotForVault, format!("{name}: {e}")))?;
    let kind = vault::iso_kind(&mut iso, families)
        .map_err(|e| fail(Code::IsoNotForVault, format!("{name}: {e}")))?
        .ok_or_else(|| {
            let known: Vec<&str> = families.iter().map(|family| family.name.as_str()).collect();
            fail(
                Code::IsoNotForVault,
                format!("{name}: none of {}", known.join(", ")),
            )
        })?;
    Ok((name, size, kind))
}

/// # Errors
/// Coded refusal before any write; then write, room or read-back failure.
pub fn vault_add(
    job: &VaultAddJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VaultAddDone, JobError> {
    let (name, size, kind) = vault_source(&job.source, job.name.as_deref(), &job.families)?;
    confirmed(job.yes, "vault add")?;
    let (mut device, info, plan) = open_vault(
        &job.target,
        Access::ReadWrite,
        job.options,
        job.elevation,
        job.trace_io,
        deps,
    )?;
    if !vault::vault_families(&*device, &plan)
        .iter()
        .any(|family| family.id == kind.family.id)
    {
        return Err(fail(
            Code::IsoNotForVault,
            format!(
                "{name}: this vault's menu does not start {} yet: bonk vault update first",
                kind.family.name
            ),
        ));
    }
    let mut file = std::fs::File::open(&job.source).map_err(|e| {
        fail(
            Code::SourceUnreadable,
            format!("{}: {e}", job.source.display()),
        )
    })?;
    let mut report = guarded(deps, progress);
    let mut advance = Advance::start(Phase::Write, size, &mut report);
    log::info!(target: "bonk::write", name = name.as_str(), bytes = size; "vault add starting");
    let added = vault::add_image(
        &mut *device,
        &plan,
        &name,
        size,
        kind.family.location,
        &mut file,
        &mut |n| advance.add(n),
    )
    .map_err(edit_error)?;
    device.flush().map_err(|e| fail(Code::WriteFailed, e))?;
    drop_cache_before_verify(&mut *device);
    let mut advance = Advance::start(Phase::Verify, size, &mut report);
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
        family: kind.family.id,
        family_name: kind.family.name,
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
