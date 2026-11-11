mod gpt;
mod mbr;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::Serialize;

use crate::block::{BlockDevice, BlockError};
use crate::bytes::{u16_le, u32_le, u64_le};

use gpt::{check_gpt, read_header};
use mbr::{check_mbr, parse_mbr};

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";

/// Minimum GPT header size (UEFI 2.x).
const GPT_HEADER_MIN: usize = 92;

/// Cap on the entry table read: 1 MiB, far above the usual 16 KiB.
const GPT_ENTRIES_LIMIT: u64 = 1 << 20;

pub(crate) const ALIGNMENT_BYTES: u64 = 1 << 20;

pub(crate) const MBR_PROTECTIVE: u8 = 0xEE;

/// CRC-32 (IEEE 802.3), as GPT uses it.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A finding of the inspection, identified by a stable code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Finding {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MbrEntry {
    pub index: u8,
    pub bootable: bool,
    /// Partition type (`0xEE`: protective MBR of a GPT).
    pub kind: u8,
    pub start_lba: u32,
    pub sectors: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Mbr {
    /// The `55 AA` bytes at the end of the first sector.
    pub signature_valid: bool,
    pub protective: bool,
    pub entries: Vec<MbrEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GptHeader {
    /// Sector where the header was read.
    pub read_at_lba: u64,
    pub revision: u32,
    pub header_size: u32,
    pub header_crc_ok: bool,
    pub my_lba: u64,
    pub alternate_lba: u64,
    pub first_usable_lba: u64,
    pub last_usable_lba: u64,
    pub disk_guid: String,
    pub entries_lba: u64,
    pub entry_count: u32,
    pub entry_size: u32,
    pub entries_crc_ok: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GptEntry {
    pub index: u32,
    pub type_guid: String,
    pub unique_guid: String,
    pub first_lba: u64,
    pub last_lba: u64,
    pub attributes: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Gpt {
    pub primary: Option<GptHeader>,
    pub backup: Option<GptHeader>,
    /// Entries of the primary table if readable, otherwise of the backup table.
    pub entries: Vec<GptEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Layout {
    pub size_bytes: u64,
    /// Logical sector size retained (512 or 4096), from the position of the GPT header.
    pub logical_block_size: u32,
    pub mbr: Option<Mbr>,
    pub gpt: Option<Gpt>,
    pub findings: Vec<Finding>,
}

/// Unreadable or inconsistent content becomes a finding, not an error.
pub fn inspect(dev: &dyn BlockDevice) -> Result<Layout, BlockError> {
    let size = dev.size();
    let mut findings = Vec::new();
    let mut note = |severity, code, message: String| {
        findings.push(Finding {
            severity,
            code,
            message,
        });
    };

    if size < 1024 {
        note(
            Severity::Error,
            "too_small",
            format!("{size} bytes: too small to hold a partition table"),
        );
        return Ok(Layout {
            size_bytes: size,
            logical_block_size: dev.block_size(),
            mbr: None,
            gpt: None,
            findings,
        });
    }

    // The GPT header is at sector 1, so 512 or 4096 bytes in.
    let logical = [512u32, 4096]
        .into_iter()
        .find(|bs| {
            u64::from(*bs) + 8 <= size
                && read_range(dev, u64::from(*bs), 8).is_ok_and(|b| b.as_slice() == GPT_SIGNATURE)
        })
        .unwrap_or(512);
    let block = u64::from(logical);

    let sector0 = read_range(dev, 0, 512)?;
    let mbr = parse_mbr(&sector0);

    let primary = read_header(dev, 1, block)?;
    let total_lba = size / block;
    let last_lba = total_lba.saturating_sub(1);
    let backup_lba = primary
        .as_ref()
        .map_or(last_lba, |(h, _)| h.alternate_lba)
        .min(last_lba);
    let backup = if backup_lba > 1 {
        read_header(dev, backup_lba, block)?
    } else {
        None
    };

    let gpt = if primary.is_some() || backup.is_some() {
        let entries = primary
            .as_ref()
            .filter(|(h, _)| h.entries_crc_ok)
            .or(backup.as_ref().filter(|(h, _)| h.entries_crc_ok))
            .or(primary.as_ref())
            .or(backup.as_ref())
            .map(|(_, entries)| entries.clone())
            .unwrap_or_default();
        Some(Gpt {
            primary: primary.as_ref().map(|(h, _)| h.clone()),
            backup: backup.as_ref().map(|(h, _)| h.clone()),
            entries,
        })
    } else {
        None
    };

    match (&mbr, &gpt) {
        (None, None) => note(
            Severity::Info,
            "no_partition_table",
            "neither MBR nor GPT: blank disk or a filesystem on the whole disk".to_owned(),
        ),
        (Some(m), None) if !m.signature_valid => note(
            Severity::Info,
            "no_partition_table",
            "MBR signature missing and no GPT".to_owned(),
        ),
        _ => {}
    }
    if let Some(m) = &mbr {
        check_mbr(m, block, size, gpt.is_some(), &mut note);
    }
    if let Some(g) = &gpt {
        check_gpt(g, block, last_lba, &mut note);
    }
    if !size.is_multiple_of(block) {
        note(
            Severity::Warning,
            "size_not_multiple_of_block",
            format!("{size} bytes are not a multiple of the {logical} byte sector"),
        );
    }

    Ok(Layout {
        size_bytes: size,
        logical_block_size: logical,
        mbr,
        gpt,
        findings,
    })
}

/// Rounds the read to the device block and returns exactly what was asked: the trait only accepts aligned
/// access.
pub fn read_range(dev: &dyn BlockDevice, offset: u64, len: u64) -> Result<Vec<u8>, BlockError> {
    let block = u64::from(dev.block_size());
    let size = dev.size();
    let out_of_range = || BlockError::OutOfRange { offset, len, size };
    // Offsets come from on-disk metadata: refuse before allocating, not after.
    let end = offset
        .checked_add(len)
        .filter(|&end| end <= size)
        .ok_or_else(out_of_range)?;
    let start = offset - offset % block;
    let aligned_end = end.div_ceil(block).saturating_mul(block).min(size);
    let mut buf = vec![0u8; usize::try_from(aligned_end - start).map_err(|_| out_of_range())?];
    dev.read_at(start, &mut buf)?;
    let skip = usize::try_from(offset - start).map_err(|_| out_of_range())?;
    buf.drain(..skip);
    buf.truncate(usize::try_from(len).map_err(|_| out_of_range())?);
    Ok(buf)
}

/// Usual text form; the first three fields are little-endian.
fn guid(b: &[u8]) -> String {
    format!(
        "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        u32_le(b, 0),
        u16_le(b, 4),
        u16_le(b, 6),
        b[8],
        b[9],
        b[10],
        b[11],
        b[12],
        b[13],
        b[14],
        b[15]
    )
}
