//! Lecture de la structure d'un disque : MBR, GPT principal et GPT de secours (CDC §9.4).
//!
//! C'est de l'**inspection** : rien n'est jamais écrit. Le résultat décrit ce qui est lu et liste les
//! constats (somme de contrôle fausse, secours absent ou mal placé, partitions qui se chevauchent...)
//! avec des codes stables, pour qu'un diagnostic puisse dire précisément ce qui ne va pas. La réparation
//! du GPT (J1) s'appuiera sur ces mêmes constats.

use schemars::JsonSchema;
use serde::Serialize;

use crate::block::{BlockDevice, BlockError};

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
/// Taille minimale d'un en-tête GPT (UEFI 2.x).
const GPT_HEADER_MIN: usize = 92;
/// Plafond de la table des partitions lue : 1 Mio, très au-delà des 16 Kio usuels (128 entrées).
const GPT_ENTRIES_LIMIT: u64 = 1 << 20;
/// Alignement recommandé du début des partitions : 1 Mio.
const ALIGNMENT_BYTES: u64 = 1 << 20;
const MBR_PROTECTIVE: u8 = 0xEE;

/// Somme de contrôle CRC-32 (IEEE 802.3) utilisée par le GPT.
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

/// Un constat de l'inspection, identifié par un code stable.
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
    /// Type de partition (`0xEE` : MBR de protection d'un GPT).
    pub kind: u8,
    pub start_lba: u32,
    pub sectors: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Mbr {
    /// Les octets `55 AA` en fin de premier secteur.
    pub signature_valid: bool,
    pub protective: bool,
    pub entries: Vec<MbrEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GptHeader {
    /// Secteur où l'en-tête a été lu.
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
    /// Entrées de la table principale si elle est lisible, sinon de la table de secours.
    pub entries: Vec<GptEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Layout {
    pub size_bytes: u64,
    /// Taille de secteur logique retenue (512 ou 4096) d'après la place de l'en-tête GPT.
    pub logical_block_size: u32,
    pub mbr: Option<Mbr>,
    pub gpt: Option<Gpt>,
    pub findings: Vec<Finding>,
}

/// Lit la structure du disque ou de l'image `dev` (lecture seule).
///
/// # Errors
/// [`BlockError`] si une lecture échoue ; un contenu illisible ou incohérent n'est pas une erreur, il
/// devient un constat.
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
            format!("{size} octets : trop petit pour contenir une table de partitions"),
        );
        return Ok(Layout {
            size_bytes: size,
            logical_block_size: dev.block_size(),
            mbr: None,
            gpt: None,
            findings,
        });
    }

    // Secteur logique : l'en-tête GPT est au secteur 1, donc à 512 ou 4096 octets du début.
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

    // --- constats
    match (&mbr, &gpt) {
        (None, None) => note(
            Severity::Info,
            "no_partition_table",
            "ni MBR ni GPT : disque vierge ou système de fichiers sur le disque entier".to_owned(),
        ),
        (Some(m), None) if !m.signature_valid => note(
            Severity::Info,
            "no_partition_table",
            "signature MBR absente et pas de GPT".to_owned(),
        ),
        _ => {}
    }
    if let Some(m) = &mbr {
        check_mbr(m, block, size, gpt.is_some(), &mut note);
    }
    if let Some(g) = &gpt {
        check_gpt(g, block, last_lba, &mbr, &mut note);
    }
    if !size.is_multiple_of(block) {
        note(
            Severity::Warning,
            "size_not_multiple_of_block",
            format!("{size} octets ne sont pas un multiple du secteur de {logical} octets"),
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

/// Lit `len` octets à `offset` en arrondissant la lecture au bloc du périphérique, et rend exactement
/// ce qui a été demandé : le contrat du trait n'accepte que des accès alignés, pas les outils de lecture.
///
/// # Errors
/// [`BlockError`] si la plage dépasse le périphérique ou si la lecture échoue.
pub fn read_range(dev: &dyn BlockDevice, offset: u64, len: u64) -> Result<Vec<u8>, BlockError> {
    let block = u64::from(dev.block_size());
    let start = offset - offset % block;
    let end = (offset + len).div_ceil(block) * block;
    let mut buf = vec![0u8; usize::try_from(end - start).unwrap_or(usize::MAX)];
    dev.read_at(start, &mut buf)?;
    let skip = usize::try_from(offset - start).unwrap_or(0);
    buf.drain(..skip);
    buf.truncate(usize::try_from(len).unwrap_or(usize::MAX));
    Ok(buf)
}

fn u16_le(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_le(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}
fn u64_le(b: &[u8], at: usize) -> u64 {
    let mut raw = [0u8; 8];
    raw.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(raw)
}

/// GUID au format texte usuel (les trois premiers champs sont en petit-boutiste).
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

fn parse_mbr(sector: &[u8]) -> Option<Mbr> {
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
    // Un premier secteur sans signature ni entrée n'est pas un MBR.
    if !signature_valid && entries.is_empty() {
        return None;
    }
    Some(Mbr {
        signature_valid,
        protective: entries.iter().any(|e| e.kind == MBR_PROTECTIVE),
        entries,
    })
}

/// Lit un en-tête GPT au secteur `lba` et la table de partitions qu'il désigne. `None` si la signature
/// n'y est pas.
type HeaderAndEntries = (GptHeader, Vec<GptEntry>);

fn read_header(
    dev: &dyn BlockDevice,
    lba: u64,
    block: u64,
) -> Result<Option<HeaderAndEntries>, BlockError> {
    let offset = lba * block;
    if offset + block > dev.size() {
        return Ok(None);
    }
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

fn parse_entries(raw: &[u8], entry_size: u32) -> Vec<GptEntry> {
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

fn check_mbr(
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
            "des entrées MBR existent mais la signature 55 AA est absente".to_owned(),
        );
    }
    if has_gpt && !mbr.protective {
        note(
            Severity::Warning,
            "gpt_protective_mbr_missing",
            "un GPT existe mais le MBR ne contient pas d'entrée de protection (type EE)".to_owned(),
        );
    }
    if mbr.protective {
        return;
    }
    let total = size / block;
    for entry in &mbr.entries {
        let end = u64::from(entry.start_lba) + u64::from(entry.sectors);
        if end > total {
            note(
                Severity::Error,
                "mbr_entry_out_of_range",
                format!(
                    "entrée MBR {} : fin au secteur {end}, le disque en compte {total}",
                    entry.index
                ),
            );
        }
    }
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
                    format!("les entrées MBR {} et {} se chevauchent", a.index, b.index),
                );
            }
        }
    }
}

fn check_gpt(
    gpt: &Gpt,
    block: u64,
    last_lba: u64,
    mbr: &Option<Mbr>,
    note: &mut impl FnMut(Severity, &'static str, String),
) {
    let _ = mbr;
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
        if !(entry.first_lba * block).is_multiple_of(ALIGNMENT_BYTES) {
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

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::*;

    pub const ESP: [u8; 16] = [
        0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E, 0xC9,
        0x3B,
    ];
    pub const LINUX: [u8; 16] = [
        0xAF, 0x3D, 0xC6, 0x0F, 0x83, 0x84, 0x72, 0x47, 0x8E, 0x79, 0x3D, 0x69, 0xD8, 0x47, 0x7D,
        0xE4,
    ];

    pub fn put_u32(buf: &mut [u8], at: usize, v: u32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }
    pub fn put_u64(buf: &mut [u8], at: usize, v: u64) {
        buf[at..at + 8].copy_from_slice(&v.to_le_bytes());
    }

    /// Image GPT valide de `sectors` secteurs de 512 octets, avec les partitions `(début, fin, type, nom)`.
    pub fn gpt_image(sectors: u64, parts: &[(u64, u64, [u8; 16], &str)]) -> Vec<u8> {
        let bs = 512usize;
        let mut img = vec![0u8; usize::try_from(sectors).unwrap_or(0) * bs];
        // MBR de protection
        img[446 + 4] = MBR_PROTECTIVE;
        put_u32(&mut img, 446 + 8, 1);
        put_u32(
            &mut img,
            446 + 12,
            u32::try_from(sectors - 1).unwrap_or(u32::MAX),
        );
        img[510] = 0x55;
        img[511] = 0xAA;
        // table : 128 entrées de 128 octets
        let mut table = vec![0u8; 128 * 128];
        for (i, (first, last, kind, name)) in parts.iter().enumerate() {
            let e = &mut table[i * 128..(i + 1) * 128];
            e[..16].copy_from_slice(kind);
            e[16..32].copy_from_slice(&[i as u8 + 1; 16]);
            put_u64(e, 32, *first);
            put_u64(e, 40, *last);
            for (j, unit) in name.encode_utf16().enumerate() {
                e[56 + j * 2..58 + j * 2].copy_from_slice(&unit.to_le_bytes());
            }
        }
        let table_crc = crc32(&table);
        let total = usize::try_from(sectors).unwrap_or(0);
        let write_header = |img: &mut Vec<u8>, at: usize, my: u64, alt: u64, entries_lba: u64| {
            let h = &mut img[at * bs..at * bs + bs];
            h[..8].copy_from_slice(GPT_SIGNATURE);
            put_u32(h, 8, 0x0001_0000);
            put_u32(h, 12, 92);
            put_u64(h, 24, my);
            put_u64(h, 32, alt);
            put_u64(h, 40, 34);
            put_u64(h, 48, sectors - 34);
            h[56..72].copy_from_slice(&[0xD1; 16]);
            put_u64(h, 72, entries_lba);
            put_u32(h, 80, 128);
            put_u32(h, 84, 128);
            put_u32(h, 88, table_crc);
            let crc = crc32(&h[..92]);
            put_u32(h, 16, crc);
        };
        write_header(&mut img, 1, 1, sectors - 1, 2);
        img[2 * bs..2 * bs + table.len()].copy_from_slice(&table);
        let backup_table = total - 33;
        img[backup_table * bs..backup_table * bs + table.len()].copy_from_slice(&table);
        write_header(&mut img, total - 1, sectors - 1, 1, sectors - 33);
        img
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{ESP, LINUX, gpt_image, put_u32, put_u64};
    use super::*;
    use crate::block::MemoryDevice;

    fn inspect_bytes(bytes: Vec<u8>) -> Result<Layout, BlockError> {
        inspect(&MemoryDevice::from_bytes(bytes, 512)?)
    }

    fn codes(layout: &Layout) -> Vec<&'static str> {
        layout.findings.iter().map(|f| f.code).collect()
    }

    #[test]
    fn crc32_valeurs_de_reference() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"a"), 0xE8B7_BE43);
    }

    #[test]
    fn gpt_valide_sans_constat_grave() -> Result<(), Box<dyn std::error::Error>> {
        let layout = inspect_bytes(gpt_image(
            4096,
            &[(2048, 2559, ESP, "EFI"), (2560, 4000, LINUX, "racine")],
        ))?;
        assert_eq!(layout.logical_block_size, 512);
        let gpt = layout.gpt.as_ref().ok_or("GPT attendu")?;
        let (primary, backup) = (
            gpt.primary.as_ref().ok_or("principal")?,
            gpt.backup.as_ref().ok_or("secours")?,
        );
        assert!(primary.header_crc_ok && primary.entries_crc_ok);
        assert!(backup.header_crc_ok && backup.entries_crc_ok);
        assert_eq!(primary.disk_guid, "D1D1D1D1-D1D1-D1D1-D1D1-D1D1D1D1D1D1");
        assert_eq!(gpt.entries.len(), 2);
        assert_eq!(
            gpt.entries[0].type_guid,
            "C12A7328-F81F-11D2-BA4B-00A0C93EC93B"
        );
        assert_eq!(gpt.entries[0].name, "EFI");
        assert_eq!(gpt.entries[1].name, "racine");
        let mbr = layout.mbr.as_ref().ok_or("MBR attendu")?;
        assert!(mbr.signature_valid && mbr.protective);
        // 2048 secteurs de 512 octets = 1 Mio : aligné ; rien d'anormal.
        assert!(
            layout.findings.iter().all(|f| f.severity == Severity::Info),
            "{:?}",
            layout.findings
        );
        Ok(())
    }

    #[test]
    fn en_tete_principal_corrompu() -> Result<(), Box<dyn std::error::Error>> {
        let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        img[512 + 40] ^= 0xFF; // premier secteur utilisable modifié sans recalcul de la somme
        let layout = inspect_bytes(img)?;
        assert!(
            codes(&layout).contains(&"gpt_primary_header_crc"),
            "{:?}",
            codes(&layout)
        );
        // Le secours reste valide : la réparation sera possible.
        assert!(!codes(&layout).contains(&"gpt_backup_header_crc"));
        Ok(())
    }

    #[test]
    fn table_principale_corrompue() -> Result<(), Box<dyn std::error::Error>> {
        let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        img[2 * 512 + 40] ^= 0x01; // un octet de la table
        let layout = inspect_bytes(img)?;
        assert!(codes(&layout).contains(&"gpt_primary_entries_crc"));
        assert!(!codes(&layout).contains(&"gpt_backup_entries_crc"));
        // Les entrées rendues viennent de la table de secours, saine.
        assert_eq!(
            layout.gpt.as_ref().map(|g| g.entries[0].last_lba),
            Some(4000)
        );
        Ok(())
    }

    #[test]
    fn secours_absent_quand_le_disque_est_plus_grand() -> Result<(), Box<dyn std::error::Error>> {
        let small = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        let mut big = small.clone();
        big.resize(8192 * 512, 0);
        let layout = inspect_bytes(big)?;
        let found = codes(&layout);
        assert!(
            found.contains(&"gpt_backup_not_at_end") || found.contains(&"gpt_backup_missing"),
            "{found:?}"
        );
        Ok(())
    }

    #[test]
    fn secours_efface_est_signale() -> Result<(), Box<dyn std::error::Error>> {
        let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        let last = 4095 * 512;
        img[last..last + 512].fill(0);
        let layout = inspect_bytes(img)?;
        assert!(codes(&layout).contains(&"gpt_backup_missing"));
        Ok(())
    }

    #[test]
    fn partitions_qui_se_chevauchent_ou_sortent_de_la_zone()
    -> Result<(), Box<dyn std::error::Error>> {
        let layout = inspect_bytes(gpt_image(
            4096,
            &[
                (2048, 3000, LINUX, "a"),
                (2900, 3500, LINUX, "b"),
                (10, 20, LINUX, "c"),
            ],
        ))?;
        let found = codes(&layout);
        assert!(found.contains(&"gpt_entries_overlap"), "{found:?}");
        assert!(found.contains(&"gpt_entry_out_of_range"), "{found:?}");
        Ok(())
    }

    #[test]
    fn partition_non_alignee_est_une_information() -> Result<(), Box<dyn std::error::Error>> {
        let layout = inspect_bytes(gpt_image(4096, &[(2049, 4000, LINUX, "x")]))?;
        let f = layout
            .findings
            .iter()
            .find(|f| f.code == "gpt_entry_unaligned");
        assert!(f.is_some_and(|f| f.severity == Severity::Info));
        Ok(())
    }

    #[test]
    fn gpt_sans_mbr_de_protection() -> Result<(), Box<dyn std::error::Error>> {
        let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        img[446 + 4] = 0;
        let layout = inspect_bytes(img)?;
        assert!(codes(&layout).contains(&"gpt_protective_mbr_missing"));
        Ok(())
    }

    #[test]
    fn mbr_classique() -> Result<(), Box<dyn std::error::Error>> {
        let mut img = vec![0u8; 4096 * 512];
        img[510] = 0x55;
        img[511] = 0xAA;
        for (i, (start, len, kind)) in [(2048u32, 1000u32, 0x0Cu8), (3048, 500, 0x83)]
            .into_iter()
            .enumerate()
        {
            let at = 446 + i * 16;
            img[at] = if i == 0 { 0x80 } else { 0 };
            img[at + 4] = kind;
            put_u32(&mut img, at + 8, start);
            put_u32(&mut img, at + 12, len);
        }
        let layout = inspect_bytes(img)?;
        assert!(layout.gpt.is_none());
        let mbr = layout.mbr.as_ref().ok_or("MBR attendu")?;
        assert_eq!(mbr.entries.len(), 2);
        assert!(mbr.entries[0].bootable && !mbr.entries[1].bootable);
        assert_eq!(
            (mbr.entries[1].kind, mbr.entries[1].start_lba),
            (0x83, 3048)
        );
        assert!(layout.findings.is_empty(), "{:?}", layout.findings);
        Ok(())
    }

    #[test]
    fn mbr_avec_entree_hors_du_disque_ou_chevauchante() -> Result<(), Box<dyn std::error::Error>> {
        let mut img = vec![0u8; 4096 * 512];
        img[510] = 0x55;
        img[511] = 0xAA;
        for (i, (start, len)) in [(100u32, 500u32), (300, 200), (4000, 500)]
            .into_iter()
            .enumerate()
        {
            let at = 446 + i * 16;
            img[at + 4] = 0x83;
            put_u32(&mut img, at + 8, start);
            put_u32(&mut img, at + 12, len);
        }
        let layout = inspect_bytes(img)?;
        let found = codes(&layout);
        assert!(
            found.contains(&"mbr_entries_overlap") && found.contains(&"mbr_entry_out_of_range"),
            "{found:?}"
        );
        Ok(())
    }

    #[test]
    fn disque_vierge_et_image_minuscule() -> Result<(), Box<dyn std::error::Error>> {
        let blank = inspect_bytes(vec![0u8; 4096 * 512])?;
        assert!(blank.mbr.is_none() && blank.gpt.is_none());
        assert_eq!(codes(&blank), ["no_partition_table"]);
        let tiny = inspect(&MemoryDevice::new(512, 512)?)?;
        assert_eq!(codes(&tiny), ["too_small"]);
        Ok(())
    }

    #[test]
    fn secteur_logique_de_4096_octets_detecte() -> Result<(), Box<dyn std::error::Error>> {
        // Un en-tête GPT au secteur 1 d'un disque à secteurs de 4096 octets se trouve à l'octet 4096.
        let sectors = 64usize;
        let bs = 4096usize;
        let mut img = vec![0u8; sectors * bs];
        let h = &mut img[bs..bs + 512];
        h[..8].copy_from_slice(GPT_SIGNATURE);
        put_u32(h, 12, 92);
        put_u64(h, 24, 1);
        put_u64(h, 32, (sectors - 1) as u64);
        put_u64(h, 72, 2);
        put_u32(h, 80, 128);
        put_u32(h, 84, 128);
        let layout = inspect_bytes(img)?;
        assert_eq!(layout.logical_block_size, 4096);
        assert!(layout.gpt.as_ref().is_some_and(|g| g.primary.is_some()));
        Ok(())
    }

    #[test]
    fn table_aberrante_ne_fait_ni_lire_hors_bornes_ni_paniquer()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        // Nombre d'entrées et taille démesurés dans l'en-tête principal.
        put_u32(&mut img, 512 + 80, u32::MAX);
        put_u32(&mut img, 512 + 84, u32::MAX);
        let layout = inspect_bytes(img)?;
        assert!(codes(&layout).contains(&"gpt_primary_entries_crc"));
        // Début de table hors du disque.
        let mut img = gpt_image(4096, &[(2048, 4000, LINUX, "x")]);
        put_u64(&mut img, 512 + 72, u64::MAX / 2);
        assert!(inspect_bytes(img).is_ok());
        Ok(())
    }
}
