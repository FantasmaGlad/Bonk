use bonk_core::BlockDevice;
use bonk_core::format::table::{self, PartitionPlan, TableKind};
use bonk_core::format::{Filesystem, FormatError, Ids, normalize_label};
use bonk_core::partition::{Severity, inspect};
use bonk_hal::platform::format_capability;
use bonk_hal::{Elevation, FilterOptions};

use super::target::{Chosen, OpenRequest, choose};
use super::target::{OpenedForWriting, open_for_writing};
use super::types::{Deps, FormatDone, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};
use super::verify::drop_cache_before_verify;
use super::write::confirm_internal;
use crate::errors::Code;

mod host;
mod native;
#[cfg(test)]
mod tests;

pub(super) use host::wait_for_partitions;
use host::*;
use native::*;

/// Above this, `exfat` rather than `fat32`: the usual tool limit and what devices expect.
const FAT32_DEFAULT_LIMIT: u64 = 32 << 30;
/// Half a minute at 250 ms per look.
const PARTITION_LOOKS: u32 = 120;

pub struct FormatJob {
    pub target: Target,
    pub filesystem: Option<Filesystem>,
    pub table: TableKind,
    pub label: Option<String>,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub force: bool,
    pub elevation: Elevation,
}

#[must_use]
pub fn default_filesystem(size: u64) -> Filesystem {
    if size <= FAT32_DEFAULT_LIMIT {
        Filesystem::Fat32
    } else {
        Filesystem::Exfat
    }
}

fn format_error(err: FormatError) -> JobError {
    fail(Code::FormatFailed, err)
}

fn check_available(fs: Filesystem, chosen: &Chosen, deps: &Deps<'_>) -> Result<(), JobError> {
    if fs.is_native() {
        return Ok(());
    }
    if chosen.disk.is_none() {
        return Err(fail(
            Code::FilesystemUnavailable,
            format!(
                "{fs} goes through system tools: not on an image file, only fat32 and exfat do"
            ),
        ));
    }
    let name = format_capability(fs);
    let capability = (deps.io.capabilities)()
        .into_iter()
        .find(|capability| capability.name == name);
    match capability {
        Some(capability) if capability.available => Ok(()),
        Some(capability) => Err(fail(
            Code::FilesystemUnavailable,
            format!(
                "{fs} unavailable: {}; --fs fat32 or exfat needs no tool",
                capability.detail.unwrap_or_default()
            ),
        )),
        None => Err(fail(
            Code::FilesystemUnavailable,
            format!("{fs} unavailable on this host"),
        )),
    }
}

fn check_table(dev: &dyn BlockDevice, part: PartitionPlan) -> Result<(), JobError> {
    let layout = inspect(dev).map_err(|e| fail(Code::FormatFailed, e))?;
    if let Some(finding) = layout
        .findings
        .iter()
        .find(|finding| finding.severity == Severity::Error)
    {
        return Err(fail(
            Code::FormatFailed,
            format!("table read back incorrect: {}", finding.message),
        ));
    }
    let in_gpt = layout.gpt.as_ref().is_some_and(|gpt| {
        gpt.entries
            .iter()
            .any(|e| e.first_lba == part.first_lba && e.last_lba == part.last_lba)
    });
    let in_mbr = layout.mbr.as_ref().is_some_and(|mbr| {
        mbr.entries
            .iter()
            .any(|e| u64::from(e.start_lba) == part.first_lba)
    });
    if in_gpt || in_mbr {
        Ok(())
    } else {
        Err(fail(
            Code::FormatFailed,
            "table read back incorrect: partition missing",
        ))
    }
}

fn write_volume(
    device: &mut dyn BlockDevice,
    kind: TableKind,
    filesystem: Filesystem,
    label: Option<&str>,
    ids: &Ids,
    cancelled: &dyn Fn() -> bool,
) -> Result<(PartitionPlan, Option<u32>), JobError> {
    // On the real drive, before the table: the plan counted 512-byte sectors, and a table written for nothing
    // would already have erased the key.
    let block = device.block_size();
    let planned = table::plan(device.size(), block, kind).map_err(format_error)?;
    let cluster_bytes =
        native_cluster_bytes(filesystem, planned.sectors(), block).map_err(format_error)?;
    let partition =
        table::write_table(device, kind, filesystem, label, ids, false).map_err(format_error)?;
    if cancelled() {
        return Err(fail(
            Code::Cancelled,
            "cancelled after writing the table: the drive must be formatted again",
        ));
    }
    write_native(device, filesystem, partition, label, ids.volume_id).map_err(format_error)?;
    device.flush().map_err(|e| fail(Code::FormatFailed, e))?;
    drop_cache_before_verify(device);
    check_table(device, partition)?;
    verify_native(device, filesystem, partition, label, ids.volume_id).map_err(format_error)?;
    Ok((partition, cluster_bytes))
}

struct Settled {
    size: u64,
    filesystem: Filesystem,
    label: Option<String>,
    planned: PartitionPlan,
    cluster_bytes: Option<u32>,
}

fn settle(job: &FormatJob, chosen: &Chosen, deps: &Deps<'_>) -> Result<Settled, JobError> {
    let size = chosen.info.size_bytes.ok_or_else(|| {
        fail(
            Code::InvalidArgument,
            "target image size unknown: give it with --size",
        )
    })?;
    let filesystem = job.filesystem.unwrap_or_else(|| default_filesystem(size));
    let label = job
        .label
        .as_deref()
        .map(|label| normalize_label(filesystem, label))
        .transpose()
        .map_err(|message| fail(Code::InvalidArgument, message))?;
    check_available(filesystem, chosen, deps)?;
    // Geometry refusal before any action, with the sector size the system reports; it is read again at open,
    // before writing the table.
    let block = chosen.block_size();
    let planned = table::plan(size, block, job.table).map_err(format_error)?;
    let cluster_bytes =
        native_cluster_bytes(filesystem, planned.sectors(), block).map_err(format_error)?;
    Ok(Settled {
        size,
        filesystem,
        label,
        planned,
        cluster_bytes,
    })
}

fn done(job: &FormatJob, s: &Settled, target: TargetInfo, unmounted: Vec<String>) -> FormatDone {
    FormatDone {
        schema_version: SCHEMA_VERSION,
        event: if job.dry_run { "plan" } else { "result" },
        dry_run: job.dry_run,
        target,
        table: job.table,
        filesystem: s.filesystem,
        filesystem_defaulted: job.filesystem.is_none(),
        file_size_limit_bytes: s.filesystem.file_size_limit(),
        label: s.label.clone(),
        partition: s.planned,
        written_by: if s.filesystem.is_native() {
            "bonk"
        } else {
            "system"
        },
        cluster_bytes: s.cluster_bytes,
        unmounted,
        verified: !job.dry_run,
    }
}

pub fn format(job: &FormatJob, deps: &Deps<'_>) -> Result<FormatDone, JobError> {
    let chosen = choose(&job.target, job.options, deps)?;
    let settled = settle(job, &chosen, deps)?;
    if job.dry_run {
        return Ok(done(job, &settled, chosen.info, chosen.mounts));
    }
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "format without confirmation: add --yes or --force (or describe the operation with --dry-run)",
        ));
    }
    confirm_internal(&chosen, job.options, job.force)?;
    format_target(job, chosen, settled, deps)
}

fn format_target(
    job: &FormatJob,
    chosen: Chosen,
    mut settled: Settled,
    deps: &Deps<'_>,
) -> Result<FormatDone, JobError> {
    let request =
        OpenRequest::read_write(job.options, job.elevation, job.trace_io, Some(settled.size));
    let OpenedForWriting {
        mut device,
        info,
        unmounted,
        ids,
        identity,
    } = open_for_writing(chosen, &job.target, &request, deps, "formatting")?;
    let filesystem = settled.filesystem;
    log::info!(
        target: "bonk::write",
        target_path = info.path.as_str(), table = job.table.name(), filesystem = filesystem.name();
        "format starting"
    );
    let (partition, cluster_bytes) = write_volume(
        &mut *device,
        job.table,
        filesystem,
        settled.label.as_deref(),
        &ids,
        &*deps.io.cancelled,
    )?;
    let block = u64::from(device.block_size());
    drop(device);
    if let Some(identity) = &identity {
        let disk = HostDisk {
            identity,
            path: &info.path,
            elevation: job.elevation,
        };
        let bytes = partition.sectors() * block;
        hand_over_to_host(&disk, bytes, filesystem, settled.label.as_deref(), deps)?;
    }
    log::info!(target: "bonk::write", filesystem = filesystem.name(); "format finished");
    settled.planned = partition;
    settled.cluster_bytes = cluster_bytes;
    Ok(done(job, &settled, info, unmounted))
}
