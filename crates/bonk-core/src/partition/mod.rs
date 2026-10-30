//! Lecture de la structure d'un disque, sans jamais écrire : MBR, GPT principal et GPT de secours.
//!
//! Un contenu illisible ou incohérent n'est pas une erreur mais un constat à code stable, pour qu'un
//! diagnostic dise précisément ce qui ne va pas ; la réparation du GPT s'appuie sur ces constats.

mod gpt;
mod mbr;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

use schemars::JsonSchema;
use serde::Serialize;

use crate::block::{BlockDevice, BlockError};

use gpt::{check_gpt, read_header};
use mbr::{check_mbr, parse_mbr};

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

/// Lit la structure de `dev`. Un contenu illisible ou incohérent devient un constat, pas une erreur.
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
        check_gpt(g, block, last_lba, &mut note);
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
