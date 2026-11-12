use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_core::BlockDevice;
use bonk_core::format::table::{self, PartitionPlan, TableKind};
use bonk_core::format::{Filesystem, FormatError, Ids, exfat, fat32, normalize_label};
use bonk_core::partition::{Severity, inspect};
use bonk_hal::platform::format_capability;
use bonk_hal::{Access, Elevation, FilterOptions, OpenTargetError, TargetIdentity};

use super::target::{
    Chosen, OpenRequest, choose, enumerate, map_open_target, open_target, reread_table,
};
use super::types::{Deps, FormatDone, JobError, SCHEMA_VERSION, Target, TargetInfo, TraceIo, fail};
use super::verify::drop_cache_before_verify;
use super::write::confirm_internal;
use crate::errors::Code;

/// Above this, `exfat` rather than `fat32`: the usual tool limit and what devices expect.
const FAT32_DEFAULT_LIMIT: u64 = 32 << 30;
/// Looks at the system while waiting for the partition or filesystem: half a minute with the 250 ms pause.
const PARTITION_LOOKS: u32 = 120;

pub struct FormatJob {
    pub target: Target,
    /// `None`: [`default_filesystem`].
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

/// `None` for host tools.
fn native_cluster_bytes(
    filesystem: Filesystem,
    sectors: u64,
    block: u32,
) -> Result<Option<u32>, FormatError> {
    Ok(match filesystem {
        Filesystem::Fat32 => Some(fat32::geometry(sectors, block)?.cluster_bytes),
        Filesystem::Exfat => Some(exfat::geometry(sectors, block)?.cluster_bytes),
        Filesystem::Ntfs | Filesystem::Ext4 => None,
    })
}

fn write_native(
    device: &mut dyn BlockDevice,
    filesystem: Filesystem,
    partition: PartitionPlan,
    label: Option<&str>,
    volume_id: u32,
) -> Result<(), FormatError> {
    match filesystem {
        Filesystem::Fat32 => fat32::write(
            device,
            &fat32::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
                bios_boot: false,
            },
        )
        .map(drop),
        Filesystem::Exfat => exfat::write(
            device,
            &exfat::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
            },
        )
        .map(drop),
        Filesystem::Ntfs | Filesystem::Ext4 => Ok(()),
    }
}

fn verify_native(
    device: &dyn BlockDevice,
    filesystem: Filesystem,
    partition: PartitionPlan,
    label: Option<&str>,
    volume_id: u32,
) -> Result<(), FormatError> {
    match filesystem {
        Filesystem::Fat32 => fat32::verify(
            device,
            &fat32::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
                bios_boot: false,
            },
        ),
        Filesystem::Exfat => exfat::verify(
            device,
            &exfat::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
            },
        ),
        Filesystem::Ntfs | Filesystem::Ext4 => Ok(()),
    }
}

/// Before any action.
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

/// Two formats must not give the same GUIDs. The only clock and randomness of formatting: `bonk-core` has
/// none, so its tests stay deterministic.
pub(super) fn fresh_seed(target: &str) -> Vec<u8> {
    use std::hash::{BuildHasher, RandomState};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let random = RandomState::new().hash_one(target);
    format!("{target}:{nanos}:{}:{random}", std::process::id()).into_bytes()
}

/// No error, and the wanted partition.
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

/// Recognized by its size: the system may keep the old table for a while, and formatting an old partition at
/// the same place would destroy the wrong area.
fn wait_for_partition(
    id: &str,
    expected_bytes: u64,
    deps: &Deps<'_>,
) -> Result<Option<String>, JobError> {
    Ok(
        wait_for_partitions(id, &[expected_bytes], deps)?
            .and_then(|paths| paths.into_iter().next()),
    )
}

/// Paths of the disk's partitions once their sizes are exactly `expected`, in any order.
pub(super) fn wait_for_partitions(
    id: &str,
    expected: &[u64],
    deps: &Deps<'_>,
) -> Result<Option<Vec<String>>, JobError> {
    let mut wanted = expected.to_vec();
    wanted.sort_unstable();
    for _ in 0..PARTITION_LOOKS {
        let disks = enumerate(deps)?;
        let found = disks.iter().find(|disk| disk.id == id).filter(|disk| {
            let mut sizes: Vec<u64> = disk.partitions.iter().map(|p| p.size_bytes).collect();
            sizes.sort_unstable();
            sizes == wanted
        });
        if let Some(disk) = found {
            return Ok(Some(
                disk.partitions.iter().map(|p| p.path.clone()).collect(),
            ));
        }
        if (deps.io.cancelled)() {
            return Err(fail(
                Code::Cancelled,
                "cancelled while waiting for the partition",
            ));
        }
        (deps.io.pause)();
    }
    Ok(None)
}

/// The host tool's output is not parsed: the system must at least recognize the result.
fn wait_for_filesystem(id: &str, fs: Filesystem, deps: &Deps<'_>) -> Result<(), JobError> {
    for _ in 0..PARTITION_LOOKS {
        let disks = enumerate(deps)?;
        let found = disks
            .iter()
            .find(|disk| disk.id == id)
            .and_then(|disk| disk.partitions.first())
            .and_then(|partition| partition.fs_type.as_deref());
        if found == Some(fs.name()) {
            return Ok(());
        }
        if (deps.io.cancelled)() {
            return Err(fail(
                Code::Cancelled,
                "cancelled while waiting for the file system",
            ));
        }
        (deps.io.pause)();
    }
    Err(fail(
        Code::FormatFailed,
        format!("the system does not recognize {fs} on the new partition"),
    ))
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

struct HostDisk<'a> {
    identity: &'a TargetIdentity,
    path: &'a str,
    elevation: Elevation,
}

/// The system rereads the table, shows the partition and formats it with its own tools.
fn hand_over_to_host(
    disk: &HostDisk<'_>,
    partition_bytes: u64,
    filesystem: Filesystem,
    label: Option<&str>,
    deps: &Deps<'_>,
) -> Result<(), JobError> {
    reread_table(Path::new(disk.path), disk.elevation, deps);
    let path = wait_for_partition(&disk.identity.id, partition_bytes, deps)?;
    if !filesystem.is_native() {
        let path = path.ok_or_else(|| {
            fail(
                Code::FormatFailed,
                "the system does not show the new partition: nothing was formatted on it",
            )
        })?;
        // The host tool writes to a path: its disk must be the chosen one (same id, serial, size). The full
        // filter is not run again: the system just mounted the new partition (Windows gives it a letter at
        // once), and that mount is ours.
        let disks = enumerate(deps)?;
        let found = disks
            .iter()
            .find(|candidate| candidate.id == disk.identity.id)
            .ok_or_else(|| map_open_target(OpenTargetError::NotFound(disk.identity.id.clone())))?;
        let actual = TargetIdentity::from(found);
        if actual != *disk.identity {
            return Err(map_open_target(OpenTargetError::IdentityChanged {
                expected: Box::new(disk.identity.clone()),
                actual: Box::new(actual),
            }));
        }
        (deps.io.format_partition)(Path::new(&path), filesystem, label, disk.elevation)
            .map_err(|e| fail(Code::FormatFailed, e))?;
        wait_for_filesystem(&disk.identity.id, filesystem, deps)?;
    } else if path.is_none() {
        // FAT32 is written and read back: the system will see it at the next plug.
        log::warn!(target: "bonk::write", "new partition not visible yet: unplug and replug the disk");
    }
    Ok(())
}

/// Choices fixed before anything is written.
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
    let seed_name = chosen.info.path.clone();
    let identity = chosen.disk.as_ref().map(TargetIdentity::from);
    let request = OpenRequest {
        options: job.options,
        access: Access::ReadWrite,
        elevation: job.elevation,
        size_hint: Some(settled.size),
        trace_io: job.trace_io,
    };
    let (mut device, info, unmounted) = open_target(chosen, &job.target, &request, deps)?;
    if (deps.io.cancelled)() {
        return Err(fail(Code::Cancelled, "cancelled before formatting"));
    }
    let ids = Ids::from_seed(&fresh_seed(&seed_name));
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

#[cfg(test)]
mod volume_tests {
    #![allow(clippy::unwrap_used)]

    use bonk_core::MemoryDevice;

    use super::*;

    fn ids() -> Ids {
        Ids::from_seed(b"essai")
    }

    /// The plan uses 512-byte sectors: on 4 KiB sectors only the real drive shows FAT32 does not fit. The
    /// refusal must come before the table.
    #[test]
    fn fat32_refuse_sur_secteurs_de_4_kio_ne_laisse_aucune_table() {
        let mut device = MemoryDevice::new(100 << 20, 4096).unwrap();
        let result = write_volume(
            &mut device,
            TableKind::Gpt,
            Filesystem::Fat32,
            None,
            &ids(),
            &|| false,
        );
        assert!(result.is_err());
        assert!(device.as_bytes().iter().all(|b| *b == 0));
    }

    /// Cancel is seen between steps: no filesystem after a cancelled table.
    #[test]
    fn l_annulation_arrete_le_formatage_entre_ses_etapes() {
        let mut device = MemoryDevice::new(100 << 20, 512).unwrap();
        let result = write_volume(
            &mut device,
            TableKind::Gpt,
            Filesystem::Fat32,
            None,
            &ids(),
            &|| true,
        );
        assert_eq!(result.err().map(|e| e.code), Some(Code::Cancelled));
        let volume = 1 << 20;
        assert!(
            device.as_bytes()[volume..volume + 512]
                .iter()
                .all(|b| *b == 0)
        );
    }
}
