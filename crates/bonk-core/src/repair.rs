//! Réparation du GPT (CDC §2, J1) : après avoir écrit une image sur un support plus grand, l'en-tête de
//! secours n'est plus à la fin du disque ; ou l'un des deux en-têtes est abîmé. On reconstruit ce qui
//! manque à partir de celui qui est valide, en gardant le GUID du disque et les partitions telles
//! quelles.
//!
//! Le support doit être inscriptible : à n'appeler que sur une cible déjà autorisée (filtre de sécurité,
//! confirmation) ou sur une image.

use std::fmt;
use std::ops::Range;

use schemars::JsonSchema;
use serde::Serialize;

use crate::block::{BlockDevice, BlockError};
use crate::partition::{GptHeader, Layout, crc32, inspect, read_range};

/// Constats d'inspection qui justifient une réparation.
const REPAIRABLE: [&str; 8] = [
    "gpt_backup_not_at_end",
    "gpt_backup_missing",
    "gpt_backup_header_crc",
    "gpt_backup_entries_crc",
    "gpt_primary_missing",
    "gpt_primary_header_crc",
    "gpt_primary_entries_crc",
    "gpt_headers_not_pointing_to_each_other",
];

/// Plages d'octets d'une image GPT que [`repair_gpt`] réécrit quand l'image est posée sur un support
/// plus grand : la taille de l'entrée du MBR de protection et, dans l'en-tête principal, la somme de
/// contrôle et les quatre positions (le sien, celui de secours, premier et dernier secteur utilisables).
/// `head` contient le début de l'image (deux secteurs au moins, 8 Kio pour couvrir les secteurs de
/// 4 Kio). Vide si ce n'est pas une image GPT.
///
/// Sert à comparer un support à son image après coup sans prendre pour une erreur ce que la réparation
/// a légitimement modifié, tout en gardant le reste de ces deux secteurs (GUID du disque...) comparé.
#[must_use]
pub fn ranges_rewritten_by_repair(head: &[u8]) -> Vec<Range<u64>> {
    for block in [512usize, 4096] {
        if head.len() < 2 * block || &head[block..block + 8] != b"EFI PART" {
            continue;
        }
        let block = block as u64;
        let mut ranges = Vec::new();
        if head[510] == 0x55 && head[511] == 0xAA {
            for entry in (0..4u64).map(|i| 446 + i * 16) {
                let at = usize::try_from(entry).unwrap_or(0);
                if head[at + 4] == 0xEE {
                    ranges.push(entry + 8..entry + 16);
                }
            }
        }
        ranges.push(block + 16..block + 20);
        ranges.push(block + 24..block + 56);
        return ranges;
    }
    Vec::new()
}

/// Vrai si l'inspection montre un défaut que [`repair_gpt`] sait corriger.
#[must_use]
pub fn needs_repair(layout: &Layout) -> bool {
    layout.gpt.is_some() && layout.findings.iter().any(|f| REPAIRABLE.contains(&f.code))
}

#[derive(Debug)]
pub enum RepairError {
    Device(BlockError),
    /// Pas de GPT sur ce support.
    NotGpt,
    /// Ni l'en-tête principal ni celui de secours n'est valide : rien ne peut servir de modèle.
    NoValidHeader,
    /// Le support est trop petit pour la table, ou une partition dépasse la zone utilisable.
    TooSmall,
}

impl fmt::Display for RepairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Device(err) => write!(f, "{err}"),
            Self::NotGpt => write!(f, "pas de table GPT sur ce support"),
            Self::NoValidHeader => write!(
                f,
                "aucun en-tête GPT valide (principal et secours sont abîmés) : rien ne peut servir de modèle"
            ),
            Self::TooSmall => write!(f, "support trop petit pour la table GPT et ses partitions"),
        }
    }
}

impl std::error::Error for RepairError {}

impl From<BlockError> for RepairError {
    fn from(err: BlockError) -> Self {
        Self::Device(err)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RepairReport {
    /// Ce qui a été réécrit, par identifiants stables.
    pub actions: Vec<&'static str>,
    pub backup_header_lba: u64,
    pub last_usable_lba: u64,
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(buf: &mut [u8], at: usize, value: u64) {
    buf[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

/// Emplacements d'un en-tête et de sa table.
struct Placement {
    my_lba: u64,
    alternate_lba: u64,
    entries_lba: u64,
}

/// Ce qui est commun aux deux en-têtes d'un disque réparé.
struct Common<'a> {
    model: &'a GptHeader,
    guid: [u8; 16],
    last_usable: u64,
    table_crc: u32,
    block: usize,
}

/// Octets d'un en-tête GPT de 92 octets, somme de contrôle comprise, dans un secteur.
fn header_bytes(common: &Common<'_>, at: &Placement) -> Vec<u8> {
    let model = common.model;
    let mut h = vec![0u8; common.block];
    h[..8].copy_from_slice(b"EFI PART");
    put_u32(&mut h, 8, model.revision);
    put_u32(&mut h, 12, 92);
    put_u64(&mut h, 24, at.my_lba);
    put_u64(&mut h, 32, at.alternate_lba);
    put_u64(&mut h, 40, model.first_usable_lba);
    put_u64(&mut h, 48, common.last_usable);
    h[56..72].copy_from_slice(&common.guid);
    put_u64(&mut h, 72, at.entries_lba);
    put_u32(&mut h, 80, model.entry_count);
    put_u32(&mut h, 84, model.entry_size);
    put_u32(&mut h, 88, common.table_crc);
    let crc = crc32(&h[..92]);
    put_u32(&mut h, 16, crc);
    h
}

/// Le GUID du disque sous forme d'octets, relu depuis l'en-tête (le texte est en champs mixtes).
fn disk_guid_bytes(
    dev: &dyn BlockDevice,
    header_lba: u64,
    block: u64,
) -> Result<[u8; 16], BlockError> {
    let sector = read_range(dev, header_lba * block, 72)?;
    let mut guid = [0u8; 16];
    guid.copy_from_slice(&sector[56..72]);
    Ok(guid)
}

/// Reconstruit les en-têtes et la table manquants ou abîmés.
///
/// # Errors
/// [`RepairError`] ; rien n'est écrit tant qu'un en-tête valide n'a pas été trouvé.
pub fn repair_gpt(dev: &mut dyn BlockDevice) -> Result<RepairReport, RepairError> {
    let layout = inspect(dev)?;
    let gpt = layout.gpt.as_ref().ok_or(RepairError::NotGpt)?;
    let block = u64::from(layout.logical_block_size);
    let block_len = usize::try_from(block).unwrap_or(512);
    let last = (dev.size() / block).saturating_sub(1);

    let good = |h: &Option<GptHeader>| h.clone().filter(|h| h.header_crc_ok && h.entries_crc_ok);
    let (primary, backup) = (good(&gpt.primary), good(&gpt.backup));
    let (model, from_primary) = match (&primary, &backup) {
        (Some(h), _) => (h.clone(), true),
        (None, Some(h)) => (h.clone(), false),
        (None, None) => return Err(RepairError::NoValidHeader),
    };
    let guid = disk_guid_bytes(dev, model.read_at_lba, block)?;
    let table_len = u64::from(model.entry_count) * u64::from(model.entry_size);
    let table = read_range(dev, model.entries_lba * block, table_len)?;
    let table_crc = crc32(&table);
    let table_sectors = table_len.div_ceil(block);

    let backup_table_lba = last
        .checked_sub(table_sectors)
        .ok_or(RepairError::TooSmall)?;
    let last_usable = backup_table_lba
        .checked_sub(1)
        .ok_or(RepairError::TooSmall)?;
    let first_usable = model.first_usable_lba.max(2 + table_sectors);
    if last_usable < first_usable || gpt.entries.iter().any(|e| e.last_lba > last_usable) {
        return Err(RepairError::TooSmall);
    }
    let model = GptHeader {
        first_usable_lba: first_usable,
        ..model
    };

    let mut padded_table = table.clone();
    padded_table.resize(
        usize::try_from(table_sectors * block).unwrap_or(table.len()),
        0,
    );
    let mut actions = Vec::new();

    // Le secours d'abord : tant qu'il n'est pas à jour, le principal reste le seul bon exemplaire.
    dev.write_at(backup_table_lba * block, &padded_table)?;
    actions.push("backup_table_written");
    let common = Common {
        model: &model,
        guid,
        last_usable,
        table_crc,
        block: block_len,
    };
    dev.write_at(
        last * block,
        &header_bytes(
            &common,
            &Placement {
                my_lba: last,
                alternate_lba: 1,
                entries_lba: backup_table_lba,
            },
        ),
    )?;
    actions.push("backup_header_written");
    if !from_primary {
        dev.write_at(2 * block, &padded_table)?;
        actions.push("primary_table_restored");
    }
    dev.write_at(
        block,
        &header_bytes(
            &common,
            &Placement {
                my_lba: 1,
                alternate_lba: last,
                entries_lba: 2,
            },
        ),
    )?;
    actions.push(if from_primary {
        "primary_header_updated"
    } else {
        "primary_header_restored"
    });

    // MBR de protection : sa taille doit couvrir le disque agrandi.
    let mut sector0 = read_range(dev, 0, block)?;
    let protective = (0..4)
        .map(|i| 446 + i * 16)
        .find(|at| sector0[at + 4] == 0xEE);
    if let Some(at) = protective.filter(|_| sector0[510] == 0x55 && sector0[511] == 0xAA) {
        let sectors = u32::try_from(last).unwrap_or(u32::MAX);
        if u32::from_le_bytes([
            sector0[at + 12],
            sector0[at + 13],
            sector0[at + 14],
            sector0[at + 15],
        ]) != sectors
        {
            put_u32(&mut sector0, at + 8, 1);
            put_u32(&mut sector0, at + 12, sectors);
            dev.write_at(0, &sector0)?;
            actions.push("protective_mbr_resized");
        }
    }
    dev.flush()?;
    Ok(RepairReport {
        actions,
        backup_header_lba: last,
        last_usable_lba: last_usable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;
    use crate::partition::testing::{ESP, LINUX, gpt_image};

    fn image() -> Vec<u8> {
        gpt_image(
            4096,
            &[(2048, 2559, ESP, "EFI"), (2560, 4000, LINUX, "racine")],
        )
    }

    fn codes(layout: &Layout) -> Vec<&'static str> {
        layout.findings.iter().map(|f| f.code).collect()
    }

    fn grown(bytes: &[u8], total_sectors: usize) -> Result<MemoryDevice, BlockError> {
        let mut big = bytes.to_vec();
        big.resize(total_sectors * 512, 0);
        MemoryDevice::from_bytes(big, 512)
    }

    #[test]
    fn disque_plus_grand_que_l_image_le_secours_revient_a_la_fin()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = grown(&image(), 8192)?;
        let before = inspect(&dev)?;
        assert!(needs_repair(&before), "{:?}", codes(&before));
        let report = repair_gpt(&mut dev)?;
        assert_eq!(report.backup_header_lba, 8191);
        assert_eq!(report.last_usable_lba, 8191 - 32 - 1);
        assert!(report.actions.contains(&"protective_mbr_resized"));
        let after = inspect(&dev)?;
        assert!(!needs_repair(&after), "{:?}", codes(&after));
        let gpt = after.gpt.as_ref().ok_or("GPT")?;
        let (p, b) = (
            gpt.primary.as_ref().ok_or("principal")?,
            gpt.backup.as_ref().ok_or("secours")?,
        );
        assert!(p.header_crc_ok && p.entries_crc_ok && b.header_crc_ok && b.entries_crc_ok);
        assert_eq!(
            (p.alternate_lba, b.my_lba, b.alternate_lba),
            (8191, 8191, 1)
        );
        assert_eq!(p.disk_guid, "D1D1D1D1-D1D1-D1D1-D1D1-D1D1D1D1D1D1");
        // Les partitions sont intactes.
        assert_eq!(gpt.entries.len(), 2);
        assert_eq!(
            (gpt.entries[1].first_lba, gpt.entries[1].last_lba),
            (2560, 4000)
        );
        assert_eq!(gpt.entries[1].name, "racine");
        // Le MBR de protection couvre tout le disque.
        let mbr = after.mbr.as_ref().ok_or("MBR")?;
        assert_eq!(mbr.entries[0].sectors, 8191);
        Ok(())
    }

    #[test]
    fn la_reparation_est_idempotente() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = grown(&image(), 8192)?;
        repair_gpt(&mut dev)?;
        let once = dev.as_bytes().to_vec();
        let report = repair_gpt(&mut dev)?;
        assert!(!report.actions.contains(&"protective_mbr_resized"));
        assert_eq!(dev.as_bytes(), once.as_slice());
        Ok(())
    }

    #[test]
    fn principal_abime_restaure_depuis_le_secours() -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = image();
        bytes[512..1024].fill(0); // en-tête principal effacé
        bytes[1024..1024 + 128].fill(0); // début de la table principale effacé
        let mut dev = MemoryDevice::from_bytes(bytes, 512)?;
        let before = inspect(&dev)?;
        assert!(codes(&before).contains(&"gpt_primary_missing"));
        let report = repair_gpt(&mut dev)?;
        assert!(
            report.actions.contains(&"primary_header_restored")
                && report.actions.contains(&"primary_table_restored")
        );
        let after = inspect(&dev)?;
        assert!(!needs_repair(&after), "{:?}", codes(&after));
        assert_eq!(after.gpt.as_ref().map(|g| g.entries.len()), Some(2));
        Ok(())
    }

    #[test]
    fn secours_abime_reconstruit_depuis_le_principal() -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = image();
        let last = 4095 * 512;
        bytes[last + 40] ^= 0xFF;
        let mut dev = MemoryDevice::from_bytes(bytes, 512)?;
        assert!(needs_repair(&inspect(&dev)?));
        repair_gpt(&mut dev)?;
        assert!(!needs_repair(&inspect(&dev)?));
        Ok(())
    }

    #[test]
    fn rien_de_valide_ou_pas_de_gpt_est_une_erreur_sans_ecriture()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut bytes = image();
        bytes[512 + 40] ^= 0xFF;
        let last = 4095 * 512;
        bytes[last + 40] ^= 0xFF;
        let mut dev = MemoryDevice::from_bytes(bytes.clone(), 512)?;
        assert!(matches!(
            repair_gpt(&mut dev),
            Err(RepairError::NoValidHeader)
        ));
        assert_eq!(dev.as_bytes(), bytes.as_slice());

        let mut blank = MemoryDevice::new(4096 * 512, 512)?;
        assert!(matches!(repair_gpt(&mut blank), Err(RepairError::NotGpt)));
        Ok(())
    }

    #[test]
    fn un_disque_trop_petit_pour_les_partitions_est_refuse()
    -> Result<(), Box<dyn std::error::Error>> {
        // Image de 4096 secteurs rognée à 3000 : la partition 2 (jusqu'à 4000) dépasse.
        let bytes = image();
        let small = MemoryDevice::from_bytes(bytes[..3000 * 512].to_vec(), 512)?;
        let mut small = small;
        // Le secours est perdu avec la coupure : seul le principal sert de modèle.
        assert!(matches!(repair_gpt(&mut small), Err(RepairError::TooSmall)));
        Ok(())
    }

    /// Garde-fou : tout octet que la réparation change dans les deux premiers secteurs est annoncé par
    /// `ranges_rewritten_by_repair`, et rien d'autre dans l'image n'est touché. Si `repair_gpt` se met à
    /// écrire ailleurs, ce test échoue au lieu de laisser la vérification passer à côté.
    #[test]
    fn les_plages_annoncees_couvrent_exactement_ce_que_la_reparation_change()
    -> Result<(), Box<dyn std::error::Error>> {
        let image = image();
        let ranges = ranges_rewritten_by_repair(&image);
        assert!(!ranges.is_empty());
        // Support quatre fois plus grand que l'image : le secours n'est plus à la fin.
        let mut dev = grown(&image, 4096 * 4)?;
        let report = repair_gpt(&mut dev)?;
        assert!(!report.actions.is_empty());
        let after = dev.as_bytes();
        let changed: Vec<u64> = (0..image.len())
            .filter(|&i| after[i] != image[i])
            .map(|i| i as u64)
            .collect();
        assert!(!changed.is_empty(), "la réparation devait modifier l'image");
        for offset in changed {
            assert!(
                ranges.iter().any(|r| r.contains(&offset)),
                "octet {offset} modifié mais non annoncé ({ranges:?})"
            );
        }
        Ok(())
    }

    #[test]
    fn une_image_sans_gpt_n_annonce_aucune_plage() {
        assert!(ranges_rewritten_by_repair(&[0u8; 8192]).is_empty());
        assert!(ranges_rewritten_by_repair(&[0u8; 100]).is_empty());
    }
}
