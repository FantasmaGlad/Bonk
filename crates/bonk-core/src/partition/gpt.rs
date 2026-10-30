//! GPT : en-têtes principal et de secours, table des partitions, et leurs constats.

use crate::block::{BlockDevice, BlockError};

use super::{
    ALIGNMENT_BYTES, GPT_ENTRIES_LIMIT, GPT_HEADER_MIN, GPT_SIGNATURE, Gpt, GptEntry, GptHeader,
    Severity, crc32, guid, read_range, u32_le, u64_le,
};

/// Lit un en-tête GPT au secteur `lba` et la table de partitions qu'il désigne. `None` si la signature
/// n'y est pas.
pub(super) type HeaderAndEntries = (GptHeader, Vec<GptEntry>);

pub(super) fn read_header(
    dev: &dyn BlockDevice,
    lba: u64,
    block: u64,
) -> Result<Option<HeaderAndEntries>, BlockError> {
    // `lba` peut venir de l'en-tête principal : aucune hypothèse sur sa grandeur.
    let Some(offset) = lba.checked_mul(block).filter(|offset| {
        offset
            .checked_add(block)
            .is_some_and(|end| end <= dev.size())
    }) else {
        return Ok(None);
    };
    let sector = read_range(dev, offset, block)?;
    if sector.len() < GPT_HEADER_MIN || &sector[..8] != GPT_SIGNATURE {
        return Ok(None);
    }
    let header_size = u32_le(&sector, 12);
    let header_len = usize::try_from(header_size).unwrap_or(usize::MAX);
    let header_crc_ok = (GPT_HEADER_MIN..=sector.len()).contains(&header_len) && {
        let mut copy = sector[..header_len].to_vec();
        copy[16..20].fill(0);
        crc32(&copy) == u32_le(&sector, 16)
    };
    let entries_lba = u64_le(&sector, 72);
    let entry_count = u32_le(&sector, 80);
    let entry_size = u32_le(&sector, 84);
    let entries_crc = u32_le(&sector, 88);

    let table_bytes = u64::from(entry_count) * u64::from(entry_size);
    let plausible = entry_size >= 128
        && entry_size.is_multiple_of(8)
        && table_bytes > 0
        && table_bytes <= GPT_ENTRIES_LIMIT
        && entries_lba.checked_mul(block).is_some_and(|start| {
            start
                .checked_add(table_bytes.div_ceil(block) * block)
                .is_some_and(|end| end <= dev.size())
        });
    let (entries, entries_crc_ok) = if plausible {
        let raw = read_range(dev, entries_lba * block, table_bytes)?;
        let ok = crc32(&raw) == entries_crc;
        (parse_entries(&raw, entry_size), ok)
    } else {
        (Vec::new(), false)
    };

    Ok(Some((
        GptHeader {
            read_at_lba: lba,
            revision: u32_le(&sector, 8),
            header_size,
            header_crc_ok,
            my_lba: u64_le(&sector, 24),
            alternate_lba: u64_le(&sector, 32),
            first_usable_lba: u64_le(&sector, 40),
            last_usable_lba: u64_le(&sector, 48),
            disk_guid: guid(&sector[56..72]),
            entries_lba,
            entry_count,
            entry_size,
            entries_crc_ok,
        },
        entries,
    )))
}

pub(super) fn parse_entries(raw: &[u8], entry_size: u32) -> Vec<GptEntry> {
    let size = usize::try_from(entry_size).unwrap_or(usize::MAX);
    raw.chunks_exact(size)
        .enumerate()
        .filter(|(_, entry)| entry[..16].iter().any(|b| *b != 0))
        .map(|(index, entry)| {
            let units: Vec<u16> = entry[56..size.min(128)]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .take_while(|unit| *unit != 0)
                .collect();
            GptEntry {
                index: u32::try_from(index).unwrap_or(u32::MAX),
                type_guid: guid(&entry[0..16]),
                unique_guid: guid(&entry[16..32]),
                first_lba: u64_le(entry, 32),
                last_lba: u64_le(entry, 40),
                attributes: u64_le(entry, 48),
                name: String::from_utf16_lossy(&units),
            }
        })
        .collect()
}

pub(super) fn check_gpt(
    gpt: &Gpt,
    block: u64,
    last_lba: u64,
    note: &mut impl FnMut(Severity, &'static str, String),
) {
    check_gpt_headers(gpt, last_lba, note);
    check_gpt_entries(gpt, block, note);
}

/// Les deux en-têtes : présence, sommes de contrôle, emplacement, cohérence entre eux.
pub(super) fn check_gpt_headers(
    gpt: &Gpt,
    last_lba: u64,
    note: &mut impl FnMut(Severity, &'static str, String),
) {
    match &gpt.primary {
        None => note(
            Severity::Error,
            "gpt_primary_missing",
            "en-tête GPT principal (secteur 1) absent ou illisible".to_owned(),
        ),
        Some(h) => {
            if !h.header_crc_ok {
                note(
                    Severity::Error,
                    "gpt_primary_header_crc",
                    "somme de contrôle de l'en-tête GPT principal fausse".to_owned(),
                );
            }
            if !h.entries_crc_ok {
                note(
                    Severity::Error,
                    "gpt_primary_entries_crc",
                    "somme de contrôle de la table des partitions principale fausse ou table illisible"
                        .to_owned(),
                );
            }
            if h.my_lba != 1 {
                note(
                    Severity::Error,
                    "gpt_primary_wrong_location",
                    format!(
                        "l'en-tête principal se dit au secteur {} au lieu de 1",
                        h.my_lba
                    ),
                );
            }
        }
    }
    match (&gpt.backup, &gpt.primary) {
        (None, _) => note(
            Severity::Warning,
            "gpt_backup_missing",
            "en-tête GPT de secours absent : image tronquée, ou disque plus grand que l'image \
             écrite (le secours n'est pas à la fin)"
                .to_owned(),
        ),
        (Some(b), primary) => {
            if !b.header_crc_ok {
                note(
                    Severity::Error,
                    "gpt_backup_header_crc",
                    "somme de contrôle de l'en-tête GPT de secours fausse".to_owned(),
                );
            }
            if !b.entries_crc_ok {
                note(
                    Severity::Error,
                    "gpt_backup_entries_crc",
                    "somme de contrôle de la table de secours fausse ou table illisible".to_owned(),
                );
            }
            if b.read_at_lba != last_lba {
                note(
                    Severity::Warning,
                    "gpt_backup_not_at_end",
                    format!(
                        "le GPT de secours est au secteur {} et non au dernier ({last_lba}) : \
                         disque agrandi après l'écriture de l'image",
                        b.read_at_lba
                    ),
                );
            }
            if let Some(p) = primary {
                if p.disk_guid != b.disk_guid {
                    note(
                        Severity::Error,
                        "gpt_guid_mismatch",
                        "le GUID du disque diffère entre le GPT principal et celui de secours"
                            .to_owned(),
                    );
                }
                if p.alternate_lba != b.read_at_lba || b.alternate_lba != 1 {
                    note(
                        Severity::Warning,
                        "gpt_headers_not_pointing_to_each_other",
                        format!(
                            "le principal désigne le secteur {} pour le secours (lu en {}), le secours \
                             désigne le secteur {} pour le principal",
                            p.alternate_lba, b.read_at_lba, b.alternate_lba
                        ),
                    );
                }
            }
        }
    }
}

/// Les partitions : bornes, zone utilisable, alignement, chevauchements.
pub(super) fn check_gpt_entries(
    gpt: &Gpt,
    block: u64,
    note: &mut impl FnMut(Severity, &'static str, String),
) {
    let usable = gpt
        .primary
        .as_ref()
        .or(gpt.backup.as_ref())
        .map(|h| (h.first_usable_lba, h.last_usable_lba));
    for entry in &gpt.entries {
        if entry.last_lba < entry.first_lba {
            note(
                Severity::Error,
                "gpt_entry_bad_range",
                format!(
                    "partition {} : fin ({}) avant le début ({})",
                    entry.index + 1,
                    entry.last_lba,
                    entry.first_lba
                ),
            );
            continue;
        }
        if let Some((first, last)) = usable
            && (entry.first_lba < first || entry.last_lba > last)
        {
            note(
                Severity::Error,
                "gpt_entry_out_of_range",
                format!(
                    "partition {} : secteurs {} à {} hors de la zone utilisable ({first} à {last})",
                    entry.index + 1,
                    entry.first_lba,
                    entry.last_lba
                ),
            );
        }
        // Valeur lue dans le support : hors de portée, elle a déjà été signalée ci-dessus.
        let start = entry.first_lba.checked_mul(block);
        if start.is_some_and(|start| !start.is_multiple_of(ALIGNMENT_BYTES)) {
            note(
                Severity::Info,
                "gpt_entry_unaligned",
                format!(
                    "partition {} : début au secteur {}, non aligné sur 1 Mio",
                    entry.index + 1,
                    entry.first_lba
                ),
            );
        }
    }
    for (i, a) in gpt.entries.iter().enumerate() {
        for b in &gpt.entries[i + 1..] {
            if a.first_lba <= b.last_lba && b.first_lba <= a.last_lba {
                note(
                    Severity::Error,
                    "gpt_entries_overlap",
                    format!(
                        "les partitions {} et {} se chevauchent",
                        a.index + 1,
                        b.index + 1
                    ),
                );
            }
        }
    }
}
