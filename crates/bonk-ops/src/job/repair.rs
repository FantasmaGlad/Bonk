use std::path::Path;

use bonk_core::BlockDevice;
use bonk_core::partition::inspect;
use bonk_core::repair::{RepairError, RepairReport, needs_repair, repair_gpt};
use bonk_hal::{Access, Elevation, FilterOptions};

use super::target::{OpenRequest, choose, open_read_only, open_target, reread_table};
use super::types::{Deps, JobError, RepairDone, SCHEMA_VERSION, Target, TraceIo, fail};
use super::verify::drop_cache_before_verify;
use super::write::confirm_internal;
use crate::errors::Code;

pub struct RepairJob {
    pub target: Target,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub elevation: Elevation,
    /// Stands for `yes` and allows an internal disk the filter options already admit. No question: a command
    /// must run with nobody at the screen.
    pub force: bool,
}

fn codes(layout: &bonk_core::partition::Layout) -> Vec<String> {
    layout.findings.iter().map(|f| f.code.to_owned()).collect()
}

fn inspected(device: &dyn BlockDevice) -> Result<bonk_core::partition::Layout, JobError> {
    inspect(device).map_err(|e| fail(Code::GptRepairFailed, e))
}

pub fn repair(job: &RepairJob, deps: &Deps<'_>) -> Result<RepairDone, JobError> {
    if job.dry_run {
        return plan(job, deps);
    }
    if !(job.yes || job.force) {
        return Err(fail(
            Code::ConfirmationRequired,
            "repair without confirmation: add --yes or --force (or describe it with --dry-run)",
        ));
    }
    let chosen = choose(&job.target, job.options, deps)?;
    confirm_internal(&chosen, job.options, job.force)?;
    let request = OpenRequest {
        options: job.options,
        access: Access::ReadWrite,
        elevation: job.elevation,
        size_hint: None,
        trace_io: job.trace_io,
    };
    let (mut device, info, _) = open_target(chosen, &job.target, &request, deps)?;
    let before = inspected(&*device)?;
    let needed = needs_repair(&before);
    let repair = if needed {
        Some(repair_and_read_back(device.as_mut())?)
    } else {
        None
    };
    let after = inspected(&*device)?;
    if needed && needs_repair(&after) {
        return Err(fail(
            Code::GptRepairFailed,
            "the repaired GPT did not read back: the drive may not keep what is written at its end",
        ));
    }
    drop(device);
    // The system rereads the changed table, otherwise it keeps the old one until replug.
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

fn plan(job: &RepairJob, deps: &Deps<'_>) -> Result<RepairDone, JobError> {
    let (device, info) =
        open_read_only(&job.target, job.options, job.elevation, job.trace_io, deps)?;
    let layout = inspected(&*device)?;
    Ok(RepairDone {
        schema_version: SCHEMA_VERSION,
        event: "plan",
        dry_run: true,
        target: info,
        needs_repair: needs_repair(&layout),
        findings_before: codes(&layout),
        repair: None,
        findings_after: None,
    })
}

/// Read back after flushing: a key that did not keep the tables must not look repaired.
fn repair_and_read_back(device: &mut dyn BlockDevice) -> Result<RepairReport, JobError> {
    let report = repair_gpt(device).map_err(|e| {
        let code = match e {
            RepairError::Device(_) => Code::WriteFailed,
            _ => Code::GptRepairFailed,
        };
        fail(code, e)
    })?;
    device.flush().map_err(|e| fail(Code::WriteFailed, e))?;
    drop_cache_before_verify(device);
    Ok(report)
}
