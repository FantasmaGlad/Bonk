use std::fmt;
use std::io::{self, Read};
use std::ops::Range;
use std::time::Instant;

use schemars::JsonSchema;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Control, Phase, Progress, chunk_for, hex, millis, progress_at, rate, read_full};
use crate::block::{BlockDevice, BlockError};

#[derive(Debug)]
pub enum VerifyError {
    Device(BlockError),
    Source(io::Error),
    Mismatch { expected: String, actual: String },
    Cancelled,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "read-back failed: {err}"),
            Self::Source(err) => write!(f, "image unreadable: {err}"),
            Self::Mismatch { expected, actual } => write!(
                f,
                "the bytes read back do not match the image (expected SHA-256 {expected}, read {actual})"
            ),
            Self::Cancelled => write!(f, "verification cancelled"),
        }
    }
}

impl std::error::Error for VerifyError {}

impl From<BlockError> for VerifyError {
    fn from(err: BlockError) -> Self {
        Self::Device(err)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct VerifyReport {
    pub bytes: u64,
    /// The system cache was dropped before the read-back. False: the read-back may have read system memory and not the drive.
    pub cache_dropped: bool,
    pub sha256: String,
    pub elapsed_ms: u64,
    pub bytes_per_second: f64,
}

pub fn verify(
    dev: &mut dyn BlockDevice,
    len: u64,
    expected_sha256: &str,
    chunk: usize,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VerifyReport, VerifyError> {
    verify_ignoring(dev, len, expected_sha256, chunk, &[], progress)
}

fn zero_ignored(buf: &mut [u8], offset: u64, ignore: &[Range<u64>]) {
    let end = offset + buf.len() as u64;
    for range in ignore {
        let (from, to) = (range.start.max(offset), range.end.min(end));
        if from < to {
            let (from, to) = (from - offset, to - offset);
            buf[usize::try_from(from).unwrap_or(0)..usize::try_from(to).unwrap_or(0)].fill(0);
        }
    }
}

/// The `ignore` ranges are zeroed on both sides: the GPT repair legitimately rewrites a few fields.
pub fn verify_ignoring(
    dev: &mut dyn BlockDevice,
    len: u64,
    expected_sha256: &str,
    chunk: usize,
    ignore: &[Range<u64>],
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VerifyReport, VerifyError> {
    let cache_dropped = dev.drop_cache().is_ok();
    let block = u64::from(dev.block_size());
    let step = chunk_for(chunk, dev.block_size());
    let mut buf = vec![0u8; step];
    let mut hasher = Sha256::new();
    let started = Instant::now();
    let mut done = 0u64;
    while done < len {
        let n = usize::try_from((len - done).min(step as u64)).unwrap_or(step);
        let padded = usize::try_from((n as u64).div_ceil(block) * block).unwrap_or(n);
        dev.read_at(done, &mut buf[..padded])?;
        zero_ignored(&mut buf[..n], done, ignore);
        hasher.update(&buf[..n]);
        done += n as u64;
        if progress(&progress_at(Phase::Verify, done, Some(len), started)) == Control::Cancel {
            return Err(VerifyError::Cancelled);
        }
    }
    let actual = hex(&hasher.finalize());
    if actual.eq_ignore_ascii_case(expected_sha256) {
        Ok(VerifyReport {
            bytes: len,
            cache_dropped,
            sha256: actual,
            elapsed_ms: millis(started),
            bytes_per_second: rate(len, started),
        })
    } else {
        Err(VerifyError::Mismatch {
            expected: expected_sha256.to_owned(),
            actual,
        })
    }
}

pub fn hash_source_ignoring(
    src: &mut dyn Read,
    size_hint: Option<u64>,
    chunk: usize,
    ignore: &[Range<u64>],
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(u64, String), VerifyError> {
    hash_reader(src, size_hint, chunk, ignore, Phase::Verify, progress)
}

fn hash_reader(
    src: &mut dyn Read,
    size_hint: Option<u64>,
    chunk: usize,
    ignore: &[Range<u64>],
    phase: Phase,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(u64, String), VerifyError> {
    let mut buf = vec![0u8; chunk.max(4096)];
    let mut hasher = Sha256::new();
    let started = Instant::now();
    let mut total = 0u64;
    loop {
        let n = read_full(src, &mut buf).map_err(VerifyError::Source)?;
        if n == 0 {
            break;
        }
        zero_ignored(&mut buf[..n], total, ignore);
        hasher.update(&buf[..n]);
        total += n as u64;
        if progress(&progress_at(phase, total, size_hint, started)) == Control::Cancel {
            return Err(VerifyError::Cancelled);
        }
    }
    Ok((total, hex(&hasher.finalize())))
}

/// The stream as is, compressed or not: publishers hash the downloaded file.
pub fn hash_stream(
    src: &mut dyn Read,
    total: Option<u64>,
    chunk: usize,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<String, VerifyError> {
    hash_reader(src, total, chunk, &[], Phase::Source, progress).map(|(_, sha256)| sha256)
}
