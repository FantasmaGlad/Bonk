use super::{MBR_PROTECTIVE, Mbr, MbrEntry, Severity, u32_le};

const CHS_HEADS: u64 = 255;
const CHS_SECTORS: u64 = 63;
/// Cylinder 1023, head 254, sector 63: what partitioning tools write past the 8 GB CHS limit.
pub(crate) const CHS_PAST_LIMIT: [u8; 3] = [0xFE, 0xFF, 0xFF];

/// The 3-byte CHS address of `lba` in an MBR entry, in the 255-head, 63-sector translation that BIOS and
/// partitioning tools share.
pub(crate) fn chs(lba: u64) -> [u8; 3] {
    let cylinder = lba / (CHS_HEADS * CHS_SECTORS);
    if cylinder > 1023 {
        return CHS_PAST_LIMIT;
    }
    let [head, ..] = (lba / CHS_SECTORS % CHS_HEADS).to_le_bytes();
    let [sector, ..] = (lba % CHS_SECTORS + 1).to_le_bytes();
    let [cylinder_low, cylinder_high, ..] = cylinder.to_le_bytes();
    [head, sector | (cylinder_high << 6), cylinder_low]
}

pub(super) fn parse_mbr(sector: &[u8]) -> Option<Mbr> {
    if sector.len() < 512 {
        return None;
    }
    let signature_valid = sector[510] == 0x55 && sector[511] == 0xAA;
    let entries: Vec<MbrEntry> = (0..4u8)
        .filter_map(|index| {
            let at = 446 + usize::from(index) * 16;
            let kind = sector[at + 4];
            (kind != 0).then(|| MbrEntry {
                index,
                bootable: sector[at] == 0x80,
                kind,
                start_lba: u32_le(sector, at + 8),
                sectors: u32_le(sector, at + 12),
            })
        })
        .collect();
    if !signature_valid && entries.is_empty() {
        return None;
    }
    Some(Mbr {
        signature_valid,
        protective: entries.iter().any(|e| e.kind == MBR_PROTECTIVE),
        entries,
    })
}

pub(super) fn check_mbr(
    mbr: &Mbr,
    block: u64,
    size: u64,
    has_gpt: bool,
    note: &mut impl FnMut(Severity, &'static str, String),
) {
    if !mbr.signature_valid {
        note(
            Severity::Warning,
            "mbr_signature_missing",
            "MBR entries exist but the 55 AA signature is missing".to_owned(),
        );
    }
    if has_gpt && !mbr.protective {
        note(
            Severity::Warning,
            "gpt_protective_mbr_missing",
            "a GPT exists but the MBR has no protective entry (type EE)".to_owned(),
        );
    }
    if mbr.protective {
        return;
    }
    check_bounds(mbr, size / block, note);
    check_overlaps(mbr, note);
}

fn check_bounds(mbr: &Mbr, total: u64, note: &mut impl FnMut(Severity, &'static str, String)) {
    for entry in &mbr.entries {
        let end = u64::from(entry.start_lba) + u64::from(entry.sectors);
        if end > total {
            note(
                Severity::Error,
                "mbr_entry_out_of_range",
                format!(
                    "MBR entry {}: ends at sector {end}, the disk has {total}",
                    entry.index
                ),
            );
        }
    }
}

fn check_overlaps(mbr: &Mbr, note: &mut impl FnMut(Severity, &'static str, String)) {
    for (i, a) in mbr.entries.iter().enumerate() {
        for b in &mbr.entries[i + 1..] {
            let (a_end, b_end) = (
                u64::from(a.start_lba) + u64::from(a.sectors),
                u64::from(b.start_lba) + u64::from(b.sectors),
            );
            if u64::from(a.start_lba) < b_end && u64::from(b.start_lba) < a_end {
                note(
                    Severity::Error,
                    "mbr_entries_overlap",
                    format!("MBR entries {} and {} overlap", a.index, b.index),
                );
            }
        }
    }
}
