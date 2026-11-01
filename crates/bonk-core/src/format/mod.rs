//! Formatage : une partition unique alignée sur 1 Mio et son système de fichiers, écrits sur
//! [`crate::BlockDevice`].
//!
//! FAT32 est écrit ici, à l'identique sur chaque hôte et sans outil externe ; exFAT, NTFS et ext4
//! restent aux outils de l'hôte, qui ne reçoivent d'ici que la table. Rien ne dépend de l'horloge ni
//! du hasard : les identifiants dérivent d'une graine donnée par l'appelant.

pub mod fat32;
pub mod fat32_tree;
pub mod table;

use std::fmt;

use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::block::BlockError;

/// Taille d'alignement des partitions et de la zone de données.
pub(crate) const ALIGNMENT: u64 = crate::partition::ALIGNMENT_BYTES;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Filesystem {
    Fat32,
    Exfat,
    Ntfs,
    Ext4,
}

impl Filesystem {
    pub const ALL: [Self; 4] = [Self::Fat32, Self::Exfat, Self::Ntfs, Self::Ext4];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Fat32 => "fat32",
            Self::Exfat => "exfat",
            Self::Ntfs => "ntfs",
            Self::Ext4 => "ext4",
        }
    }

    /// Taille maximale d'un fichier, quand elle peut gêner sur une clé : FAT32 code la taille sur 32 bits.
    #[must_use]
    pub fn file_size_limit(self) -> Option<u64> {
        (self == Self::Fat32).then_some(u64::from(u32::MAX))
    }

    /// Bonk l'écrit-il lui-même, sans outil de l'hôte ?
    #[must_use]
    pub fn is_native(self) -> bool {
        self == Self::Fat32
    }

    /// Longueur maximale d'une étiquette : en caractères ASCII pour FAT32, en unités UTF-16 pour exFAT et
    /// NTFS, en octets pour ext4.
    fn label_limit(self) -> usize {
        match self {
            Self::Fat32 | Self::Exfat => 11,
            Self::Ntfs => 32,
            Self::Ext4 => 16,
        }
    }
}

impl fmt::Display for Filesystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Valide une étiquette pour ce système de fichiers et la met sous sa forme écrite (FAT32 : majuscules).
pub fn normalize_label(fs: Filesystem, label: &str) -> Result<String, String> {
    let limit = fs.label_limit();
    let length = match fs {
        Filesystem::Fat32 => label.len(),
        Filesystem::Exfat | Filesystem::Ntfs => label.encode_utf16().count(),
        Filesystem::Ext4 => label.len(),
    };
    if label.is_empty() || length > limit {
        return Err(format!("label '{label}': 1 to {limit} characters for {fs}"));
    }
    if label.chars().any(char::is_control) {
        return Err("label: control character refused".to_owned());
    }
    if fs != Filesystem::Fat32 {
        return Ok(label.to_owned());
    }
    const FORBIDDEN: &str = "\"*+,./:;<=>?[\\]|";
    if !label.is_ascii() || label.chars().any(|c| FORBIDDEN.contains(c)) {
        return Err(format!(
            "label '{label}': FAT32 only accepts ASCII, without {FORBIDDEN}"
        ));
    }
    Ok(label.to_ascii_uppercase())
}

/// Identifiants d'un support formaté, dérivés d'une graine : à graine égale, résultat égal aux octets près.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ids {
    pub disk_guid: [u8; 16],
    pub partition_guid: [u8; 16],
    pub disk_signature: u32,
    pub volume_id: u32,
}

/// GUID aléatoire au sens de la RFC 4122 (version 4), dans l'ordre d'octets du GPT.
fn guid(bytes: &[u8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    out.copy_from_slice(&bytes[..16]);
    out[7] = (out[7] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

impl Ids {
    #[must_use]
    pub fn from_seed(seed: &[u8]) -> Self {
        let digest = Sha256::digest(seed);
        let next = Sha256::digest(digest);
        Self {
            disk_guid: guid(&digest[..16]),
            partition_guid: guid(&digest[16..]),
            disk_signature: u32::from_le_bytes([next[0], next[1], next[2], next[3]]),
            volume_id: u32::from_le_bytes([next[4], next[5], next[6], next[7]]),
        }
    }
}

#[derive(Debug)]
pub enum FormatError {
    Device(BlockError),
    TooSmall(String),
    TooLarge(String),
    /// La relecture ne donne pas ce qui a été écrit.
    Mismatch(String),
    /// Un nom de fichier que FAT32 ne peut pas porter.
    Name(String),
    /// Lecture de la source pendant la copie.
    Source(std::io::Error),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "{err}"),
            Self::TooSmall(what)
            | Self::TooLarge(what)
            | Self::Mismatch(what)
            | Self::Name(what) => f.write_str(what),
            Self::Source(err) => write!(f, "reading the image: {err}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<BlockError> for FormatError {
    fn from(err: BlockError) -> Self {
        Self::Device(err)
    }
}

/// Écrit des zéros sur `[offset, offset + len[`, par morceaux : les tables FAT d'un grand support ne
/// tiennent pas en mémoire d'un seul tenant.
pub(crate) fn zero_range(
    dev: &mut dyn crate::block::BlockDevice,
    offset: u64,
    len: u64,
) -> Result<(), BlockError> {
    let block = u64::from(dev.block_size());
    let step = ALIGNMENT.div_ceil(block) * block;
    let zeros = vec![0u8; usize::try_from(step).unwrap_or(1 << 20)];
    let mut done = 0;
    while done < len {
        let n = (len - done).min(step);
        dev.write_at(
            offset + done,
            &zeros[..usize::try_from(n).unwrap_or(zeros.len())],
        )?;
        done += n;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etiquettes_selon_le_systeme_de_fichiers() {
        assert_eq!(
            normalize_label(Filesystem::Fat32, "Ma Cle").as_deref(),
            Ok("MA CLE")
        );
        for refusee in ["", "DOUZE_LETTRES", "A/B", "CLÉ", "a\tb"] {
            assert!(
                normalize_label(Filesystem::Fat32, refusee).is_err(),
                "{refusee}"
            );
        }
        assert_eq!(
            normalize_label(Filesystem::Exfat, "Clé").as_deref(),
            Ok("Clé")
        );
        assert!(normalize_label(Filesystem::Ext4, "dix-sept-octets!!").is_err());
        assert!(normalize_label(Filesystem::Ntfs, &"x".repeat(32)).is_ok());
        assert!(normalize_label(Filesystem::Ntfs, &"x".repeat(33)).is_err());
    }

    #[test]
    fn identifiants_stables_pour_une_graine_et_guid_de_version_4() {
        let ids = Ids::from_seed(b"graine");
        assert_eq!(ids, Ids::from_seed(b"graine"));
        assert_ne!(ids, Ids::from_seed(b"autre"));
        assert_ne!(ids.disk_guid, ids.partition_guid);
        for g in [ids.disk_guid, ids.partition_guid] {
            assert_eq!(g[7] >> 4, 4);
            assert_eq!(g[8] >> 6, 2);
        }
    }
}
