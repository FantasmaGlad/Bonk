pub mod exfat;
pub mod exfat_edit;
pub mod exfat_read;
pub mod exfat_tree;
pub mod fat32;
pub mod fat32_tree;
pub mod table;
mod tree;

pub use tree::NewFile;

use std::fmt;

use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::block::BlockError;

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

    /// FAT32 stores file sizes on 32 bits.
    #[must_use]
    pub fn file_size_limit(self) -> Option<u64> {
        (self == Self::Fat32).then_some(u64::from(u32::MAX))
    }

    #[must_use]
    pub fn is_native(self) -> bool {
        matches!(self, Self::Fat32 | Self::Exfat)
    }

    /// ASCII characters for FAT32, UTF-16 units for exFAT and NTFS, bytes for ext4.
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

/// Returns the label as written (upper case for FAT32).
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

/// Derived from a seed: same seed, same bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ids {
    pub disk_guid: [u8; 16],
    pub partition_guid: [u8; 16],
    pub disk_signature: u32,
    pub volume_id: u32,
}

/// RFC 4122 version 4, in GPT byte order.
pub(crate) fn guid(bytes: &[u8]) -> [u8; 16] {
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
    Mismatch(String),
    Name(String),
    Source(std::io::Error),
    /// A combination the drive cannot carry, such as BIOS code on GPT.
    Unsupported(String),
    /// Content read from the drive that Bonk does not understand.
    Invalid(String),
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "{err}"),
            Self::TooSmall(what)
            | Self::TooLarge(what)
            | Self::Mismatch(what)
            | Self::Name(what)
            | Self::Unsupported(what)
            | Self::Invalid(what) => f.write_str(what),
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

/// In chunks: the FATs of a large drive do not fit in memory at once.
pub(crate) fn zero_range(
    dev: &mut dyn crate::block::BlockDevice,
    offset: u64,
    len: u64,
) -> Result<(), BlockError> {
    let block = u64::from(dev.block_size());
    let step = ALIGNMENT.div_ceil(block) * block;
    let zeros = vec![0u8; step as usize];
    let mut done = 0;
    while done < len {
        let n = (len - done).min(step);
        dev.write_at(offset + done, &zeros[..n as usize])?;
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
