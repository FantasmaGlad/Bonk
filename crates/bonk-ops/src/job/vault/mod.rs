use bonk_core::BlockDevice;
use bonk_core::vault::{self, VaultPlan};
use bonk_hal::{Access, Elevation, FilterOptions};

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
