//! Modèle de disque commun aux systèmes.
//!
//! Les adaptateurs d'OS le remplissent de **faits bruts**, sans interprétation. La décision de
//! proposer, cacher ou interdire un disque revient à la fonction pure de [`crate::safety`], testable
//! sans matériel et identique sur tous les systèmes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bus connecting the disk to the host.
///
/// Safety relies on the bus and not on the "removable" flag: on Windows, a USB hard disk is often reported as "fixed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Usb,
    /// SD or MMC card reader.
    Sd,
    /// **External** Thunderbolt: an adapter must report this bus only for a connected enclosure, not for an internal SSD going through the same interface.
    Thunderbolt,
    Sata,
    Nvme,
    Scsi,
    /// Bus of a host virtual disk (loop, zram, VM disk).
    Virtual,
    /// Bus the adapter could not determine: treated as internal, to be safe.
    Unknown,
}

impl Bus {
    /// Les bus sur lesquels un disque est proposé par défaut.
    #[must_use]
    pub(crate) fn is_removable(self) -> bool {
        matches!(self, Self::Usb | Self::Sd | Self::Thunderbolt)
    }
}

/// Nature of the disk from the host point of view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    Physical,
    /// Loop device backed by a file.
    Loop,
    Zram,
    /// Virtual machine disk mounted by the host.
    HostVirtual,
}

/// Partition (or volume) of a disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Partition {
    /// System path, such as `/dev/sdb1`.
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Filesystem or container signature as reported by the OS (`ext4`, `vfat`, `swap`, `LVM2_member`, `linux_raid_member`, `zfs_member`, `crypto_LUKS`...).
    #[serde(default)]
    pub fs_type: Option<String>,
    pub size_bytes: u64,
    /// Current mount points (`/`, `/run/media/alice/KEY`, `E:\`).
    #[serde(default)]
    pub mount_points: Vec<String>,
    /// Devices stacked on top (RAID `md0`, LVM or unlocked encrypted volume `dm-0`).
    #[serde(default)]
    pub holders: Vec<String>,
    /// Host system volume: `%SystemDrive%` on Windows, boot volume on macOS, root on Linux.
    #[serde(default)]
    pub is_system_volume: bool,
    /// EFI system partition **the host booted from** (not every ESP-type partition).
    #[serde(default)]
    pub is_active_esp: bool,
}

/// Disk as reported by the enumeration of an OS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    /// Stable identifier between two connections: serial number or `by-id` path. Never `/dev/sdX` alone, which changes between connections.
    pub id: String,
    /// Current system path, for display and opening.
    pub path: String,
    pub model: String,
    #[serde(default)]
    pub serial: Option<String>,
    pub size_bytes: u64,
    pub bus: Bus,
    pub kind: DiskKind,
    /// System "removable" flag, **not reliable**: informative only.
    #[serde(default)]
    pub removable_flag: bool,
    /// Signature on the whole disk, without a partition table (bare LVM physical volume...).
    #[serde(default)]
    pub fs_type: Option<String>,
    /// Devices stacked directly on the whole disk.
    #[serde(default)]
    pub holders: Vec<String>,
    #[serde(default)]
    pub partitions: Vec<Partition>,
}
