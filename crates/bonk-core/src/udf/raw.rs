use super::UdfError;
use crate::bytes::{get_u16, get_u32, get_u64};

/// Optical media sector. The main anchor is always at sector 256.
pub(super) const SECTOR: u64 = 2048;
pub(super) const ANCHOR_SECTOR: u64 = 256;
pub(super) const MAX_DESCRIPTORS: u64 = 4096;
pub(super) const MAX_DEPTH: usize = 32;
pub(super) const MAX_ENTRIES: usize = 1_000_000;
pub(super) const MAX_DIRECTORY_BYTES: u64 = 64 << 20;

pub(super) const TAG_PARTITION: u16 = 5;
pub(super) const TAG_LOGICAL_VOLUME: u16 = 6;
pub(super) const TAG_TERMINATOR: u16 = 8;
pub(super) const TAG_ANCHOR: u16 = 2;
pub(super) const TAG_FILE_SET: u16 = 256;
pub(super) const TAG_FILE_ID: u16 = 257;
pub(super) const TAG_FILE_ENTRY: u16 = 261;
pub(super) const TAG_EXTENDED_FILE_ENTRY: u16 = 266;

pub(super) const ICB_DIRECTORY: u8 = 4;
pub(super) const FID_DIRECTORY: u8 = 0x02;
pub(super) const FID_DELETED: u8 = 0x04;
pub(super) const FID_PARENT: u8 = 0x08;

pub(super) const OUTSIDE: UdfError = UdfError::Malformed("field outside the descriptor");

pub(super) fn u16_at(data: &[u8], at: usize) -> Result<u16, UdfError> {
    get_u16(data, at).ok_or(OUTSIDE)
}

pub(super) fn u32_at(data: &[u8], at: usize) -> Result<u32, UdfError> {
    get_u32(data, at).ok_or(OUTSIDE)
}

pub(super) fn u64_at(data: &[u8], at: usize) -> Result<u64, UdfError> {
    get_u64(data, at).ok_or(OUTSIDE)
}

/// Id of a descriptor whose tag checksum is valid; the version byte is not checked.
pub(super) fn tag_id(sector: &[u8]) -> Option<u16> {
    let head = sector.get(..16)?;
    let sum = head
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
    (sum == head[4]).then(|| u16::from_le_bytes([head[0], head[1]]))
}

#[derive(Clone, Copy)]
pub(super) struct LongAd {
    pub(super) block: u32,
}

pub(super) fn long_ad(data: &[u8], at: usize) -> Result<LongAd, UdfError> {
    Ok(LongAd {
        block: u32_at(data, at + 4)?,
    })
}

/// The extent type is in the two high bits of the length; 0: recorded.
pub(super) fn recorded_len(raw: u32) -> Result<u64, UdfError> {
    if raw >> 30 == 0 {
        Ok(u64::from(raw))
    } else {
        Err(UdfError::Malformed("file extent not recorded"))
    }
}

/// First byte 8: Latin-1; 16: big-endian UTF-16.
pub(super) fn decode_name(raw: &[u8]) -> Result<String, UdfError> {
    let name = match raw.split_first() {
        Some((8, rest)) => rest.iter().map(|b| char::from(*b)).collect(),
        Some((16, rest)) if rest.len().is_multiple_of(2) => {
            let units = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c));
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| UdfError::Malformed("invalid file name"))?
        }
        _ => return Err(UdfError::Malformed("invalid file name")),
    };
    // A name from the image becomes a path: it must not escape.
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(UdfError::Malformed("invalid file name"));
    }
    Ok(name)
}
