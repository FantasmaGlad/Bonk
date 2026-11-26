use std::collections::BTreeMap;

use bonk_core::BlockDevice;
use bonk_core::vault::{self, VaultPlan};
use bonk_hal::{Access, Elevation, FilterOptions};
use schemars::JsonSchema;
use serde::Serialize;

use super::target::{OpenRequest, choose, open_target};
use super::types::{Deps, JobError, Target, TargetInfo, TraceIo, fail};
use crate::errors::Code;

mod edit;
mod init;
mod list;
#[cfg(test)]
mod tests;
mod update;

pub use edit::*;
pub use init::*;
pub use list::*;
pub use update::*;

/// The GRUB written to the boot partition, as the signed catalog names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GrubInfo {
    pub version: String,
    pub build: String,
    /// SHA-256 of each EFI image written, by processor (`x86_64`, `aarch64`).
    pub efi: BTreeMap<String, String>,
    /// SHA-256 of `boot.img` and `core.img`, GRUB for BIOS machines; empty when the catalog offers none.
    pub bios: BTreeMap<String, String>,
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
