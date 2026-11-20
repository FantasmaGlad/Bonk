use std::path::PathBuf;

use bonk_core::format::Filesystem;
use bonk_core::format::table::TableKind;
use bonk_hal::{Elevation, FilterOptions};

use crate::errors::Code;
use bonk_ops::job::{FormatJob, RepairJob, Target, TraceIo, VerifyJob, WriteJob, WriteMode};

use crate::fetch;
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

/// What a write starts from: a catalog image brings its hash, file name and mode.
pub(in crate::serve) struct WriteSource {
    pub(in crate::serve) source: PathBuf,
    pub(in crate::serve) sha256: Option<String>,
    pub(in crate::serve) file_name: Option<String>,
    pub(in crate::serve) hybrid: bool,
}

/// A catalog image takes its hash and file name from the signed catalog; no client value may replace them.
pub(in crate::serve) fn write_source(p: &TransferParams) -> Result<WriteSource, Failure> {
    let Some(id) = &p.image else {
        return Ok(WriteSource {
            source: p.source()?,
            sha256: p.sha256.clone(),
            file_name: p.file_name.clone(),
            hybrid: false,
        });
    };
    if p.sha256.is_some() || p.sha256_file.is_some() || p.file_name.is_some() {
        return Err(failure(
            Code::InvalidArgument,
            "image takes its hash and file name from the signed catalog: sha256, sha256_file and file_name are refused",
        ));
    }
    let found = crate::catalog::image_source(
        id,
        p.catalog_url.as_deref(),
        p.cache_dir.as_deref(),
        crate::catalog::Destination::Drive,
    )
    .map_err(|e| failure(e.code, e.message))?;
    Ok(WriteSource {
        source: found.url,
        sha256: Some(found.sha256),
        file_name: found.file_name,
        hybrid: found.hybrid,
    })
}

pub(in crate::serve) fn write_task(a: DiskArgs, p: TransferParams) -> Result<Task, Failure> {
    let found = write_source(&p)?;
    Ok(Task::Write(
        Box::new(WriteJob {
            target: a.target,
            source: found.source,
            options: a.options,
            verify: !p.no_verify,
            repair_gpt: !p.no_repair_gpt,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            expected_sha256: found.sha256,
            mode: crate::catalog::mode_for(a.mode, found.hybrid),
            table: a.table,
            elevation: a.elevation,
            force: p.force,
        }),
        fetch::Fetch {
            sha256_file: p.sha256_file,
            cache_dir: p.cache_dir,
            file_name: found.file_name,
        },
    ))
}
