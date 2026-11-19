use std::fmt;
use std::io::{self, Read};
use std::time::Instant;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::block::{BlockDevice, BlockError};

/// 4 MiB, like the `dd bs=4M` that throughput is compared to.
pub const DEFAULT_CHUNK: usize = 4 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Download of the image, before touching the drive.
    Download,
    /// Waiting for system authorization.
    Authorize,
    /// Check of the image hash, before touching the drive.
    Source,
    /// Writing the image.
    Write,
    /// Flushing buffers to the drive.
    Flush,
    /// Read-back and comparison.
    Verify,
}

/// Progress of an operation, reported at each block.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Progress {
    pub phase: Phase,
    pub bytes_done: u64,
    /// Total when known (raw image, or write already done for the verification).
    pub bytes_total: Option<u64>,
    pub bytes_per_second: f64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    Continue,
    Cancel,
}

#[derive(Debug)]
pub enum WriteError {
    Device(BlockError),
    Source(io::Error),
    /// Nothing was written past the end of the device.
    TooLarge {
        device_size: u64,
        needed: Option<u64>,
    },
    Cancelled {
        bytes_written: u64,
    },
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "write failed: {err}"),
            Self::Source(err) => write!(f, "image unreadable: {err}"),
            Self::TooLarge {
                device_size,
                needed: Some(needed),
            } => write!(
                f,
                "the image is {needed} bytes, the drive {device_size} bytes"
            ),
            Self::TooLarge { device_size, .. } => write!(
                f,
                "the decompressed image exceeds the drive ({device_size} bytes): write stopped at its end"
            ),
            Self::Cancelled { bytes_written } => {
                write!(f, "cancelled after {bytes_written} bytes written")
            }
        }
    }
}

impl std::error::Error for WriteError {}

impl From<BlockError> for WriteError {
    fn from(err: BlockError) -> Self {
        Self::Device(err)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct WriteReport {
    /// Image bytes written.
    pub bytes: u64,
    /// Bytes really sent to the drive: the image rounded up to the sector, the remainder is zero.
    pub padded_bytes: u64,
    /// SHA-256 of the image bytes, in hexadecimal.
    pub sha256: String,
    pub elapsed_ms: u64,
    pub bytes_per_second: f64,
}

/// Lowercase, as hashes are written in sums files and the catalog.
#[must_use]
pub fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

fn rate(bytes: u64, started: Instant) -> f64 {
    let secs = started.elapsed().as_secs_f64();
    if secs > 0.0 { bytes as f64 / secs } else { 0.0 }
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn progress_at(phase: Phase, done: u64, total: Option<u64>, started: Instant) -> Progress {
    Progress {
        phase,
        bytes_done: done,
        bytes_total: total,
        bytes_per_second: rate(done, started),
        elapsed_ms: millis(started),
    }
}

fn chunk_for(chunk: usize, block: u32) -> usize {
    let block = block as usize;
    chunk.max(block).div_ceil(block) * block
}

fn read_full(src: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match src.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err),
        }
    }
    Ok(filled)
}

/// If `size_hint` exceeds the drive, nothing is touched.
///
/// # Errors
/// On cancel or I/O error the drive is partially written and must be rewritten whole.
pub fn write_image(
    src: &mut dyn Read,
    dev: &mut dyn BlockDevice,
    size_hint: Option<u64>,
    chunk: usize,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteReport, WriteError> {
    if let Some(needed) = size_hint
        && needed > dev.size()
    {
        return Err(WriteError::TooLarge {
            device_size: dev.size(),
            needed: Some(needed),
        });
    }
    let block = u64::from(dev.block_size());
    let mut buf = vec![0u8; chunk_for(chunk, dev.block_size())];
    let mut hasher = Sha256::new();
    let started = Instant::now();
    let (mut written, mut offset) = (0u64, 0u64);

    if progress(&progress_at(Phase::Write, 0, size_hint, started)) == Control::Cancel {
        return Err(WriteError::Cancelled { bytes_written: 0 });
    }
    loop {
        let n = read_full(src, &mut buf).map_err(WriteError::Source)?;
        if n == 0 {
            break;
        }
        let padded = (n as u64).div_ceil(block) * block;
        buf[n..usize::try_from(padded).unwrap_or(n)].fill(0);
        if offset + padded > dev.size() {
            return Err(WriteError::TooLarge {
                device_size: dev.size(),
                needed: None,
            });
        }
        dev.write_at(offset, &buf[..usize::try_from(padded).unwrap_or(n)])?;
        hasher.update(&buf[..n]);
        written += n as u64;
        offset += padded;
        if progress(&progress_at(Phase::Write, written, size_hint, started)) == Control::Cancel {
            return Err(WriteError::Cancelled {
                bytes_written: written,
            });
        }
    }
    let _ = progress(&progress_at(Phase::Flush, written, Some(written), started));
    dev.flush()?;
    Ok(WriteReport {
        bytes: written,
        padded_bytes: offset,
        sha256: hex(&hasher.finalize()),
        elapsed_ms: millis(started),
        bytes_per_second: rate(written, started),
    })
}

mod verify;
pub use verify::*;

#[cfg(test)]
mod tests;
