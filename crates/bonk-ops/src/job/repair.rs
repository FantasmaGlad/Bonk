//! Réparer le GPT.

use std::path::Path;

use bonk_core::partition::inspect;
use bonk_core::repair::{RepairError, needs_repair, repair_gpt};
use bonk_hal::{Access, Elevation, FilterOptions};

use super::target::{OpenRequest, choose, open_read_only, open_target, reread_table};
use super::types::{Deps, JobError, RepairDone, SCHEMA_VERSION, Target, TraceIo, fail};
use super::write::confirm_internal;
use crate::errors::Code;

pub struct RepairJob {
    pub target: Target,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub elevation: Elevation,
    /// Tient lieu de `yes` et autorise un disque interne déjà admis par les options du filtre. Sans
    /// aucune question : une commande doit pouvoir tourner sans personne devant l'écran.
    pub force: bool,
}

pub fn repair(job: &RepairJob, deps: &Deps<'_>) -> Result<RepairDone, JobError> {
    let codes = |layout: &bonk_core::partition::Layout| -> Vec<String> {
        layout.findings.iter().map(|f| f.code.to_owned()).collect()
    };
    if job.dry_run || !(job.yes || job.force) {
        if !job.dry_run {
            return Err(fail(
                Code::ConfirmationRequired,
                "repair without confirmation: add --yes or --force (or describe it with --dry-run)",
            ));
        }
        let (device, info) =
            open_read_only(&job.target, job.options, job.elevation, job.trace_io, deps)?;
        let layout = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
        return Ok(RepairDone {
            schema_version: SCHEMA_VERSION,
            event: "plan",
            dry_run: true,
            target: info,
            needs_repair: needs_repair(&layout),
            findings_before: codes(&layout),
            repair: None,
            findings_after: None,
        });
    }
    let chosen = choose(&job.target, job.options, deps)?;
    confirm_internal(&chosen, job.options, job.force)?;
    let (mut device, info, _) = open_target(
        chosen,
        &job.target,
        &OpenRequest {
            options: job.options,
            access: Access::ReadWrite,
            elevation: job.elevation,
            size_hint: None,
            trace_io: job.trace_io,
        },
        deps,
    )?;
    let before = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    let needed = needs_repair(&before);
    let repair = if needed {
        Some(repair_gpt(&mut *device).map_err(|e| {
            let code = match e {
                RepairError::Device(_) => Code::WriteFailed,
                _ => Code::GptRepairFailed,
            };
            fail(code, e)
        })?)
    } else {
        None
    };
    if needed {
        // Relue après vidage : une clé qui n'a pas gardé les tables ne doit pas passer pour réparée.
        device.flush().map_err(|e| fail(Code::WriteFailed, e))?;
        let _ = device.drop_cache();
    }
    let after = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    if needed && needs_repair(&after) {
        return Err(fail(
            Code::GptRepairFailed,
            "the repaired GPT did not read back: the drive may not keep what is written at its end",
        ));
    }
    drop(device);
    // Le système relit la table qu'on vient de changer, sinon il garde l'ancienne jusqu'au rebranchement.
    if needed && matches!(job.target, Target::Disk(_)) {
        reread_table(Path::new(&info.path), job.elevation, deps);
    }
    Ok(RepairDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        dry_run: false,
        target: info,
        needs_repair: needed,
        findings_before: codes(&before),
        repair,
        findings_after: Some(codes(&after)),
    })
}
