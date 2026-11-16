use std::path::Path;

use bonk_core::format::Filesystem;
use bonk_hal::{Elevation, OpenTargetError, TargetIdentity};

use super::PARTITION_LOOKS;
use crate::errors::Code;
use crate::job::target::{enumerate, map_open_target, reread_table};
use crate::job::types::{Deps, JobError, fail};

/// Recognized by its size: the system may keep the old table for a while, and formatting an old partition at
/// the same place would destroy the wrong area.
pub(super) fn wait_for_partition(
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
pub(in crate::job) fn wait_for_partitions(
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
pub(super) fn wait_for_filesystem(
    id: &str,
    fs: Filesystem,
    deps: &Deps<'_>,
) -> Result<(), JobError> {
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

pub(super) struct HostDisk<'a> {
    pub(super) identity: &'a TargetIdentity,
    pub(super) path: &'a str,
    pub(super) elevation: Elevation,
}

/// The system rereads the table, shows the partition and formats it with its own tools.
pub(super) fn hand_over_to_host(
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
