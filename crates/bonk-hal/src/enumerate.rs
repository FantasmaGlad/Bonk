use std::error::Error;
use std::fmt;
use std::io;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::disk::Disk;

#[derive(Debug)]
pub enum EnumerateError {
    Io {
        what: String,
        source: io::Error,
    },
    Malformed {
        what: String,
        detail: String,
    },
    Unsupported {
        os: &'static str,
    },
    /// Android without root: the only way to a drive is the one termux-usb hands over.
    ThroughTermuxUsb,
}

impl fmt::Display for EnumerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { what, source } => write!(f, "could not read {what}: {source}"),
            Self::Malformed { what, detail } => write!(f, "unexpected {what}: {detail}"),
            Self::Unsupported { os } => write!(f, "disk enumeration is not available yet on {os}"),
            Self::ThroughTermuxUsb => f.write_str(
                "on Android, a drive is reached through termux-usb (Termux:API): `termux-usb -l` lists the \
                 drives, then `termux-usb -r -E -e 'bonk list' /dev/bus/usb/001/002` with the path it gives",
            ),
        }
    }
}

impl Error for EnumerateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub fn enumerate_disks() -> Result<Vec<Disk>, EnumerateError> {
    crate::platform::current().enumerate()
}

/// udev database on Linux; captures replayed on any host still name it.
pub const UDEV_DATA: &str = "/run/udev/data";

/// A read of a system source, as it happened.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceRead {
    pub path: String,
    /// `file` (content read), `dir` (list of entries) or `link` (resolved link: the content is the canonical target relative to `/sys`).
    pub kind: String,
    pub ok: bool,
    pub bytes: Option<usize>,
    pub error: Option<String>,
    pub micros: u64,
    /// Content (file) or entries (directory), truncated; only when recording.
    pub content: Option<String>,
}

pub(crate) fn captured<T: serde::de::DeserializeOwned>(
    reads: &[SourceRead],
    source: &str,
) -> Option<Result<T, EnumerateError>> {
    let read = reads.iter().find(|read| read.path == source)?;
    let parsed = read
        .content
        .as_deref()
        .ok_or_else(|| "no content".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()));
    Some(parsed.map_err(|detail| EnumerateError::Malformed {
        what: source.to_owned(),
        detail,
    }))
}

pub fn enumerate_disks_recorded() -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
    crate::platform::current().enumerate_recorded()
}

/// From a capture (`bonk diagnose --capture`), without touching the system: replays a support case elsewhere
/// (`--fixture`). Windows and macOS captures replay anywhere; Linux ones on Linux.
pub fn enumerate_disks_from_capture(reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
    if let Some(result) = crate::windows_facts::from_capture(reads) {
        return result;
    }
    if let Some(result) = crate::macos_facts::from_capture(reads) {
        return result;
    }
    crate::platform::current().enumerate_from_capture(reads)
}
