use std::fmt;

mod cache;
mod keys;
mod load;
#[cfg(test)]
mod tests;
mod types;
mod verify;

pub(crate) use cache::hash_matches;
#[cfg(test)]
pub(crate) use cache::hex;
pub use cache::{FetchSized, asset_matches, asset_path, icon_path};
pub use keys::TRUSTED_KEYS;
pub(crate) use load::replace;
pub use load::{Loaded, MANIFEST, SIGNATURE, load, saved};
pub use types::*;
pub use verify::verify;

pub const SCHEMA_VERSION: u32 = 2;

pub const DEFAULT_BASE: &str = "https://catalog.bonk-os.app/v2/";

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
