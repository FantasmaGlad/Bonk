use std::path::PathBuf;

use bonk_core::format::Filesystem;
use bonk_core::format::table::TableKind;
use bonk_hal::{Elevation, FilterOptions};

use crate::errors::Code;
use bonk_ops::job::{FormatJob, RepairJob, Target, TraceIo, VerifyJob, WriteJob, WriteMode};

use crate::serve::Config;
use crate::serve::params::TransferParams;
use crate::serve::protocol::{Failure, failure};

use super::*;

/// Every parameter is parsed whatever the operation, so a malformed one is always refused.
pub(in crate::serve) struct DiskArgs {
    options: FilterOptions,
    target: Target,
    trace: Option<TraceIo>,
    mode: WriteMode,
    elevation: Elevation,
    filesystem: Option<Filesystem>,
    table: TableKind,
}

pub(in crate::serve) fn disk_args(
    kind: DiskKind,
    p: &TransferParams,
    cfg: &Config,
) -> Result<DiskArgs, Failure> {
    let options = match kind {
        DiskKind::Verify => p.options(&cfg.view),
        DiskKind::Write | DiskKind::Repair | DiskKind::Format => p.write_options(),
    };
    let default_table = match kind {
        DiskKind::Format => TableKind::FORMAT_DEFAULT,
        DiskKind::Write | DiskKind::Verify | DiskKind::Repair => TableKind::WINDOWS_DEFAULT,
    };
    Ok(DiskArgs {
        options,
        target: p.target()?,
        trace: p.trace()?,
        mode: p.mode()?,
        elevation: p.elevation()?,
        filesystem: p.filesystem()?,
        table: p.table(default_table)?,
    })
}

pub(in crate::serve) fn prepare_disk(
    kind: DiskKind,
    p: TransferParams,
    cfg: &Config,
) -> Result<Task, Failure> {
    let a = disk_args(kind, &p, cfg)?;
    Ok(match kind {
        DiskKind::Write => write_task(a, p)?,
        DiskKind::Verify => Task::Verify(VerifyJob {
            target: a.target,
            source: p.source()?,
            options: a.options,
            trace_io: a.trace,
            elevation: a.elevation,
        }),
        DiskKind::Repair => Task::Repair(RepairJob {
            target: a.target,
            options: a.options,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            elevation: a.elevation,
            force: p.force,
        }),
        DiskKind::Format => Task::Format(FormatJob {
            target: a.target,
            filesystem: a.filesystem,
            table: a.table,
            label: p.label,
            options: a.options,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            force: p.force,
            elevation: a.elevation,
        }),
    })
}

/// A catalog image takes its hash from the signed catalog; no client hash may replace it.
pub(in crate::serve) fn write_source(
    p: &TransferParams,
) -> Result<(PathBuf, Option<String>), Failure> {
    let Some(id) = &p.image else {
        return Ok((p.source()?, p.sha256.clone()));
    };
    if p.sha256.is_some() || p.sha256_file.is_some() {
        return Err(failure(
            Code::InvalidArgument,
            "image takes its hash from the signed catalog: sha256 and sha256_file are refused",
        ));
    }
    let (source, hash) =
        crate::catalog::image_source(id, p.catalog_url.as_deref(), p.cache_dir.as_deref())
            .map_err(|e| failure(e.code, e.message))?;
    Ok((source, Some(hash)))
}

pub(in crate::serve) fn write_task(a: DiskArgs, p: TransferParams) -> Result<Task, Failure> {
    let (source, expected_sha256) = write_source(&p)?;
    Ok(Task::Write(
        Box::new(WriteJob {
            target: a.target,
            source,
            options: a.options,
            verify: !p.no_verify,
            repair_gpt: !p.no_repair_gpt,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            expected_sha256,
            mode: a.mode,
            table: a.table,
            elevation: a.elevation,
            force: p.force,
        }),
        p.sha256_file,
        p.cache_dir,
    ))
}
