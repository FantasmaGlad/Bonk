use std::fmt;

mod cache;
mod load;
#[cfg(test)]
mod tests;
mod types;
mod verify;

pub(crate) use cache::hash_matches;
#[cfg(test)]
pub(crate) use cache::hex;
pub use cache::{FetchSized, asset_matches, asset_path, icon_path};
pub(crate) use load::replace;
pub use load::{Loaded, MANIFEST, SIGNATURE, load, saved};
pub use types::*;
pub use verify::verify;

pub const SCHEMA_VERSION: u32 = 2;

pub const DEFAULT_BASE: &str = "https://catalog.bonk-os.app/v2/";

/// Current key, then an offline backup that outlives its loss without a new release.
pub const TRUSTED_KEYS: [[u8; 32]; 2] = [
    [
        0x0a, 0xa9, 0xf4, 0x66, 0x76, 0x3f, 0x33, 0x45, 0x0a, 0xe8, 0x21, 0x17, 0xf3, 0x00, 0xd4,
        0x90, 0x32, 0x62, 0xa8, 0x4a, 0x21, 0x11, 0xae, 0x31, 0x3d, 0xf1, 0xe8, 0xb1, 0x5a, 0xb1,
        0x67, 0xd4,
    ],
    [
        0xed, 0xce, 0x26, 0x90, 0xbc, 0xab, 0xa1, 0x8c, 0x90, 0xfd, 0x01, 0x0a, 0x80, 0x87, 0xab,
        0x3a, 0x9d, 0x78, 0x85, 0x5d, 0xae, 0x4c, 0xb0, 0xe6, 0xb3, 0x36, 0xf7, 0x17, 0x30, 0xc1,
        0xe0, 0xee,
    ],
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    BadSignature,
    Unreadable(String),
    UnsupportedVersion(u32),
    Expired { expires_unix: u64 },
    Rollback { sequence: u64, seen: u64 },
    InvalidImage { id: String, reason: &'static str },
    InvalidGrub(&'static str),
    Unavailable(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSignature => f.write_str("catalog signature does not match"),
            Self::Unreadable(detail) => write!(f, "unreadable catalog: {detail}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "catalog format {version} needs a newer version of Bonk")
            }
            Self::Expired { expires_unix } => {
                write!(f, "catalog expired (at unix time {expires_unix})")
            }
            Self::Rollback { sequence, seen } => {
                write!(
                    f,
                    "catalog {sequence} is older than the one already seen ({seen})"
                )
            }
            Self::InvalidImage { id, reason } => write!(f, "catalog image '{id}': {reason}"),
            Self::InvalidGrub(reason) => write!(f, "catalog GRUB entry: {reason}"),
            Self::Unavailable(detail) => write!(f, "catalog unavailable: {detail}"),
        }
    }
}

impl std::error::Error for CatalogError {}

#[derive(Debug, Clone, Copy)]
pub struct Trust<'a> {
    pub keys: &'a [[u8; 32]],
    pub now_unix: u64,
    pub seen_sequence: u64,
}
