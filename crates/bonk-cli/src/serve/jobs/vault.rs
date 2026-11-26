use bonk_core::format::table::TableKind;
use bonk_hal::Elevation;

use bonk_ops::job::{
    self, Target, TraceIo, VaultAddJob, VaultInitJob, VaultListJob, VaultRemoveJob, VaultUpdateJob,
};

use crate::serve::Config;
use crate::serve::params::TransferParams;
use crate::serve::protocol::{Failure, failure};

use super::*;

pub(in crate::serve) fn vault_grub(
    p: &TransferParams,
    cfg: &Config,
) -> Result<(bonk_core::vault::VaultBoot, job::GrubInfo), Failure> {
    let source = crate::vault::GrubSource {
        grub_dir: p.grub_dir.clone(),
        catalog_url: p.catalog_url.clone(),
        cache_dir: p.cache_dir.clone(),
    };
    crate::vault::grub(&source, &cfg.catalog).map_err(|e| failure(e.code, e.message))
}

pub(in crate::serve) fn prepare_vault(
    kind: VaultKind,
    p: &TransferParams,
    cfg: &Config,
) -> Result<Task, Failure> {
    let (target, trace_io, elevation) = (p.target()?, p.trace()?, p.elevation()?);
    Ok(match kind {
        VaultKind::List => Task::VaultList(VaultListJob {
            target,
            options: p.options(&cfg.view),
            trace_io,
            elevation,
        }),
        VaultKind::Init => vault_init_task(target, trace_io, elevation, p, cfg)?,
        VaultKind::Add => Task::VaultAdd(VaultAddJob {
            target,
            source: required(p.iso.clone(), "iso")?,
            name: None,
            options: p.write_options(),
            trace_io,
            yes: p.yes,
            elevation,
            families: bonk_core::vault::default_families(),
        }),
        VaultKind::Remove => Task::VaultRemove(VaultRemoveJob {
            target,
            name: required(p.name.clone(), "name")?,
            options: p.write_options(),
            trace_io,
            yes: p.yes,
            elevation,
        }),
        VaultKind::Update => {
            let (boot, grub) = vault_grub(p, cfg)?;
            Task::VaultUpdate(Box::new(VaultUpdateJob {
                target,
                boot,
                grub: Some(grub),
                options: p.write_options(),
                trace_io,
                dry_run: p.dry_run || !p.yes,
                yes: p.yes,
                elevation,
            }))
        }
    })
}

pub(in crate::serve) fn vault_init_task(
    target: Target,
    trace_io: Option<TraceIo>,
    elevation: Elevation,
    p: &TransferParams,
    cfg: &Config,
) -> Result<Task, Failure> {
    // A plan or a refusal for want of confirmation writes nothing: no GRUB needed.
    let (boot, grub) = if p.dry_run || !(p.yes || p.force) {
        (bonk_core::vault::VaultBoot::default(), None)
    } else {
        let (boot, grub) = vault_grub(p, cfg)?;
        (boot, Some(grub))
    };
    Ok(Task::VaultInit(Box::new(VaultInitJob {
        target,
        boot,
        grub,
        table: p.table(TableKind::VAULT_DEFAULT)?,
        options: p.write_options(),
        trace_io,
        dry_run: p.dry_run,
        yes: p.yes,
        force: p.force,
        elevation,
    })))
}
