//! Modèle de disque commun aux trois systèmes.
//!
//! Chaque adaptateur d'OS (UDisks2/sysfs, SetupAPI, DiskArbitration) remplit ces structures avec des
//! **faits bruts** : ce que le système rapporte, sans interprétation. La décision « ce disque est-il
//! proposé, caché ou interdit » est prise ailleurs, par la fonction pure de [`crate::safety`], pour
//! qu'elle soit testable sur des jeux de données sans matériel et identique sur tous les systèmes.

use serde::{Deserialize, Serialize};

/// Bus par lequel le disque est relié à l'hôte.
///
/// La sécurité se fonde sur le bus et non sur le drapeau « amovible » : sous Windows, un disque dur
/// USB est souvent signalé « fixe » (CDC §3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bus {
    Usb,
    /// Lecteur de carte SD ou MMC.
    Sd,
    /// Thunderbolt **externe** : l'adaptateur ne rapporte ce bus que pour un boîtier branché, pas
    /// pour un SSD interne qui passerait par la même interface.
    Thunderbolt,
    Sata,
    Nvme,
    Scsi,
    /// Bus d'un disque virtuel de l'hôte (loop, zram, disque de VM).
    Virtual,
    /// Bus que l'adaptateur n'a pas su déterminer : traité comme interne, par prudence.
    Unknown,
}

impl Bus {
    /// Les bus sur lesquels un disque est proposé par défaut (CDC §3.2).
    #[must_use]
    pub fn is_removable(self) -> bool {
        matches!(self, Self::Usb | Self::Sd | Self::Thunderbolt)
    }
}

/// Nature du disque du point de vue de l'hôte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiskKind {
    Physical,
    /// Périphérique loop adossé à un fichier.
    Loop,
    Zram,
    /// Disque virtuel d'une machine virtuelle monté par l'hôte.
    HostVirtual,
}

/// Partition (ou volume) d'un disque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Partition {
    /// Chemin système, par exemple `/dev/sdb1`.
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
    /// Signature de système de fichiers ou de conteneur telle que la rapporte l'OS (`ext4`, `vfat`,
    /// `swap`, `LVM2_member`, `linux_raid_member`, `zfs_member`, `crypto_LUKS`...).
    #[serde(default)]
    pub fs_type: Option<String>,
    pub size_bytes: u64,
    /// Points de montage en vigueur (`/`, `/run/media/alice/CLE`, `E:\`).
    #[serde(default)]
    pub mount_points: Vec<String>,
    /// Périphériques empilés par-dessus (RAID `md0`, LVM ou volume chiffré déverrouillé `dm-0`).
    #[serde(default)]
    pub holders: Vec<String>,
    /// Volume système de l'hôte : `%SystemDrive%` sous Windows, volume de démarrage sous macOS,
    /// racine sous Linux.
    #[serde(default)]
    pub is_system_volume: bool,
    /// Partition système EFI **dont l'hôte a démarré** (pas toute partition de type ESP).
    #[serde(default)]
    pub is_active_esp: bool,
}

/// Disque tel que rapporté par l'énumération d'un OS.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disk {
    /// Identifiant stable entre deux branchements : numéro de série ou chemin `by-id` (CDC §3.3).
    /// Jamais `/dev/sdX` seul, qui change d'un branchement à l'autre.
    pub id: String,
    /// Chemin système courant, pour l'affichage et l'ouverture.
    pub path: String,
    pub model: String,
    #[serde(default)]
    pub serial: Option<String>,
    pub size_bytes: u64,
    pub bus: Bus,
    pub kind: DiskKind,
    /// Drapeau « amovible » du système, **non fiable** : informatif seulement.
    #[serde(default)]
    pub removable_flag: bool,
    /// Signature portée par le disque entier, sans table de partitions (volume physique LVM nu...).
    #[serde(default)]
    pub fs_type: Option<String>,
    /// Périphériques empilés directement sur le disque entier.
    #[serde(default)]
    pub holders: Vec<String>,
    #[serde(default)]
    pub partitions: Vec<Partition>,
}
