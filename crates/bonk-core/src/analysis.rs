//! Analyse d'une image avant écriture : que deviendra une clé écrite octet pour octet ?
//!
//! Écrire et relire ne dit rien de cela : la relecture compare le support à l'image, pas l'image à ce
//! qu'elle devrait être. Une ISO non hybride ou un téléchargement coupé donneraient une clé « vérifiée »
//! qui ne démarre pas. L'analyse lit le début de l'image, ne dépend ni du nom du fichier ni de la
//! distribution, et juge uniquement ce qui s'y trouve.

use schemars::JsonSchema;
use serde::Serialize;

use crate::block::{DEFAULT_BLOCK_SIZE, MemoryDevice};
use crate::partition::{Layout, MBR_PROTECTIVE, inspect};

/// Octets du début de l'image à fournir à [`analyze`] : couvre les descripteurs ISO 9660 et les tables
/// de partitions, placées assez loin par certaines images hybrides.
pub const HEAD_LEN: usize = 1 << 20;

const ISO_SECTOR: usize = 2048;
const ISO_DESCRIPTORS: usize = 16 * ISO_SECTOR;
/// Assez de descripteurs pour atteindre ceux d'UDF, qui suivent ceux d'ISO 9660.
const ISO_DESCRIPTORS_SCANNED: usize = 16;
const ESP_GUID: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
const MBR_ESP: u8 = 0xEF;

/// Ce qu'est l'image, écrite telle quelle sur une clé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageKind {
    /// ISO 9660 qui porte aussi un amorçage de disque : démarre écrite telle quelle.
    HybridIso,
    /// Image de disque avec amorçage BIOS ou partition système EFI.
    BootableDisk,
    /// Table de partitions sans amorçage de PC : carte ARM, ou simples données.
    DataDisk,
    /// ISO 9660 sans amorçage de disque : démarre sur un DVD, pas écrite telle quelle sur une clé.
    PlainIso,
    /// Rien de reconnu.
    Unknown,
}

/// Résultat de l'analyse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ImageReport {
    pub kind: ImageKind,
    /// Le micrologiciel BIOS trouvera du code d'amorçage.
    pub bios: bool,
    /// Le micrologiciel UEFI trouvera une partition système EFI.
    pub uefi: bool,
    /// Présence de descripteurs UDF, typique des ISO d'installation de Windows.
    pub udf: bool,
    /// Taille que la structure de l'image annonce, si elle en annonce une.
    pub declared_bytes: Option<u64>,
    /// L'image est plus courte que ce qu'elle annonce (`None` : taille réelle inconnue, image compressée).
    pub truncated: Option<bool>,
}

impl ImageReport {
    /// Peut-on attendre un démarrage ou un usage de la clé écrite telle quelle ?
    #[must_use]
    pub fn is_writable_as_is(&self) -> bool {
        matches!(
            self.kind,
            ImageKind::HybridIso | ImageKind::BootableDisk | ImageKind::DataDisk
        )
    }
}

struct Iso {
    declared_bytes: Option<u64>,
    udf: bool,
}

/// Lit les descripteurs de volume ISO 9660 (ECMA-119 §8) et cherche ceux d'UDF (ECMA-167).
fn read_iso(head: &[u8]) -> Option<Iso> {
    let primary = head.get(ISO_DESCRIPTORS..ISO_DESCRIPTORS + ISO_SECTOR)?;
    if primary[0] != 1 || &primary[1..6] != b"CD001" {
        return None;
    }
    let space = u64::from(u32::from_le_bytes([
        primary[80],
        primary[81],
        primary[82],
        primary[83],
    ]));
    let block = u64::from(u16::from_le_bytes([primary[128], primary[129]]));
    let udf = (0..ISO_DESCRIPTORS_SCANNED).any(|i| {
        head.get(ISO_DESCRIPTORS + i * ISO_SECTOR + 1..ISO_DESCRIPTORS + i * ISO_SECTOR + 6)
            .is_some_and(|id| id == b"NSR02" || id == b"NSR03")
    });
    Some(Iso {
        declared_bytes: Some(space * block).filter(|n| *n > 0),
        udf,
    })
}

fn has_bios_code(head: &[u8], layout: &Layout) -> bool {
    layout.mbr.as_ref().is_some_and(|m| m.signature_valid)
        && head
            .get(..440)
            .is_some_and(|code| code.iter().any(|b| *b != 0))
}

fn has_esp(layout: &Layout) -> bool {
    let in_gpt = layout.gpt.as_ref().is_some_and(|g| {
        g.entries
            .iter()
            .any(|e| e.type_guid.eq_ignore_ascii_case(ESP_GUID))
    });
    in_gpt
        || layout
            .mbr
            .as_ref()
            .is_some_and(|m| m.entries.iter().any(|e| e.kind == MBR_ESP))
}

fn has_partition_table(layout: &Layout) -> bool {
    layout.gpt.as_ref().is_some_and(|g| g.primary.is_some())
        || layout
            .mbr
            .as_ref()
            .is_some_and(|m| m.signature_valid && !m.entries.is_empty())
}

/// Fin du contenu d'après les tables : le GPT annonce où est son en-tête de secours, le MBR où finissent
/// ses partitions, en secteurs de la taille que révèle le GPT (512 octets sans GPT : le MBR seul ne la
/// dit pas). Le MBR de protection d'un GPT est ignoré : il couvre tout un disque, pas l'image.
fn declared_by_tables(layout: &Layout) -> Option<u64> {
    let block = u64::from(layout.logical_block_size);
    let by_gpt = layout
        .gpt
        .as_ref()
        .and_then(|g| g.primary.as_ref())
        .filter(|h| h.header_crc_ok && h.alternate_lba > 1)
        // Valeur lue dans l'image : un débordement passerait pour une petite taille.
        .map(|h| h.alternate_lba.saturating_add(1).saturating_mul(block));
    let by_mbr = layout
        .mbr
        .as_ref()
        .filter(|m| m.signature_valid)
        .and_then(|m| {
            m.entries
                .iter()
                .filter(|e| e.kind != MBR_PROTECTIVE && e.sectors > 0)
                .map(|e| (u64::from(e.start_lba) + u64::from(e.sectors)) * block)
                .max()
        });
    by_gpt.or(by_mbr)
}

/// Analyse le début d'une image. `head` : ses premiers octets (au plus [`HEAD_LEN`] suffisent) ;
/// `real_len` : sa taille décompressée quand on la connaît.
#[must_use]
pub fn analyze(head: &[u8], real_len: Option<u64>) -> ImageReport {
    let block = DEFAULT_BLOCK_SIZE as usize;
    let usable = head.len() - head.len() % block;
    let layout = MemoryDevice::from_bytes(head[..usable].to_vec(), DEFAULT_BLOCK_SIZE)
        .ok()
        .and_then(|dev| inspect(&dev).ok());
    let iso = read_iso(head);
    let (bios, uefi, table) = layout.as_ref().map_or((false, false, false), |l| {
        (has_bios_code(head, l), has_esp(l), has_partition_table(l))
    });
    let bootable = bios || uefi;
    let kind = match (&iso, bootable, table) {
        (Some(_), true, _) => ImageKind::HybridIso,
        (Some(_), false, _) => ImageKind::PlainIso,
        (None, true, _) => ImageKind::BootableDisk,
        (None, false, true) => ImageKind::DataDisk,
        (None, false, false) => ImageKind::Unknown,
    };
    let by_iso = iso.as_ref().and_then(|i| i.declared_bytes);
    let by_tables = layout.as_ref().and_then(declared_by_tables);
    let declared_bytes = by_iso.into_iter().chain(by_tables).max();
    ImageReport {
        kind,
        bios,
        uefi,
        udf: iso.as_ref().is_some_and(|i| i.udf),
        declared_bytes,
        truncated: real_len.map(|len| declared_bytes.is_some_and(|needed| len < needed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Image de 1 Mio remplie de zéros, avec les retouches voulues.
    fn blank() -> Vec<u8> {
        vec![0u8; HEAD_LEN]
    }

    fn with_iso(image: &mut [u8], sectors: u32) {
        let primary = &mut image[ISO_DESCRIPTORS..ISO_DESCRIPTORS + ISO_SECTOR];
        primary[0] = 1;
        primary[1..6].copy_from_slice(b"CD001");
        primary[80..84].copy_from_slice(&sectors.to_le_bytes());
        primary[128..130].copy_from_slice(&2048u16.to_le_bytes());
    }

    fn with_mbr_entry(image: &mut [u8], kind: u8, start: u32, sectors: u32) {
        image[446 + 4] = kind;
        image[446 + 8..446 + 12].copy_from_slice(&start.to_le_bytes());
        image[446 + 12..446 + 16].copy_from_slice(&sectors.to_le_bytes());
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    fn with_boot_code(image: &mut [u8]) {
        image[0] = 0xEB;
        image[510] = 0x55;
        image[511] = 0xAA;
    }

    #[test]
    fn des_donnees_quelconques_ne_sont_pas_une_image_amorcable() {
        let report = analyze(&[0x5A; 4096], Some(4096));
        assert_eq!(report.kind, ImageKind::Unknown);
        assert!(!report.is_writable_as_is());
        assert_eq!(analyze(&[], None).kind, ImageKind::Unknown);
    }

    #[test]
    fn une_iso_sans_amorcage_de_disque_est_refusee() {
        let mut image = blank();
        with_iso(&mut image, 512);
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.kind, ImageKind::PlainIso);
        assert!(!report.is_writable_as_is());
        assert_eq!(report.truncated, Some(false));
    }

    #[test]
    fn une_iso_udf_est_signalee() {
        let mut image = blank();
        with_iso(&mut image, 512);
        image[ISO_DESCRIPTORS + 2 * ISO_SECTOR + 1..ISO_DESCRIPTORS + 2 * ISO_SECTOR + 6]
            .copy_from_slice(b"NSR03");
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.kind, ImageKind::PlainIso);
        assert!(report.udf);
    }

    #[test]
    fn une_iso_avec_code_d_amorcage_est_hybride_en_bios() {
        let mut image = blank();
        with_iso(&mut image, 512);
        with_boot_code(&mut image);
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.kind, ImageKind::HybridIso);
        assert!(report.bios);
        assert!(!report.uefi);
    }

    #[test]
    fn une_partition_systeme_efi_donne_l_amorcage_uefi() {
        let mut image = blank();
        with_mbr_entry(&mut image, MBR_ESP, 64, 2048);
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.kind, ImageKind::BootableDisk);
        assert!(report.uefi);
        assert!(!report.bios);
    }

    #[test]
    fn une_table_sans_amorcage_est_une_image_de_donnees_acceptee() {
        let mut image = blank();
        with_mbr_entry(&mut image, 0x0C, 8192, 100);
        let report = analyze(&image, None);
        assert_eq!(report.kind, ImageKind::DataDisk);
        assert!(report.is_writable_as_is());
        assert_eq!(report.truncated, None);
    }

    #[test]
    fn une_image_plus_courte_que_ce_qu_elle_annonce_est_tronquee() {
        let mut image = blank();
        with_iso(&mut image, 10_000);
        with_boot_code(&mut image);
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.declared_bytes, Some(10_000 * 2048));
        assert_eq!(report.truncated, Some(true));
        // Sans la taille réelle (image compressée), on ne prétend rien.
        assert_eq!(analyze(&image, None).truncated, None);
        // Une image exactement de la taille annoncée, ou plus longue (bourrage), est entière.
        assert_eq!(analyze(&image, Some(10_000 * 2048)).truncated, Some(false));
        assert_eq!(analyze(&image, Some(10_001 * 2048)).truncated, Some(false));
    }

    #[test]
    fn une_partition_mbr_qui_depasse_l_image_la_dit_tronquee() {
        let mut image = blank();
        with_mbr_entry(&mut image, MBR_ESP, 64, 1 << 20);
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.truncated, Some(true));
    }

    #[test]
    fn le_mbr_de_protection_ne_compte_pas_comme_taille() {
        let mut image = blank();
        with_mbr_entry(&mut image, MBR_PROTECTIVE, 1, u32::MAX);
        let report = analyze(&image, Some(HEAD_LEN as u64));
        assert_eq!(report.declared_bytes, None);
        assert_eq!(report.truncated, Some(false));
    }

    /// En-tête GPT valide qui annonce son secours à `alternate_lba` : la valeur vient de l'image.
    fn with_gpt_alternate(image: &mut [u8], alternate_lba: u64) {
        with_mbr_entry(image, MBR_PROTECTIVE, 1, 2047);
        let entries = vec![0u8; 128 * 128];
        let mut h = vec![0u8; 92];
        h[..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&0x10000u32.to_le_bytes());
        h[12..16].copy_from_slice(&92u32.to_le_bytes());
        h[24..32].copy_from_slice(&1u64.to_le_bytes());
        h[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
        h[40..48].copy_from_slice(&34u64.to_le_bytes());
        h[48..56].copy_from_slice(&2000u64.to_le_bytes());
        h[72..80].copy_from_slice(&2u64.to_le_bytes());
        h[80..84].copy_from_slice(&128u32.to_le_bytes());
        h[84..88].copy_from_slice(&128u32.to_le_bytes());
        h[88..92].copy_from_slice(&crate::partition::crc32(&entries).to_le_bytes());
        let crc = crate::partition::crc32(&h);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
        image[512..604].copy_from_slice(&h);
    }

    /// Image à secteurs de 4 Kio dont l'en-tête GPT est invalide : seul le MBR hybride donne la taille,
    /// et ses entrées se comptent en secteurs de 4 Kio, pas de 512 octets.
    #[test]
    fn le_mbr_d_une_image_a_secteurs_de_4_kio_se_compte_en_4_kio() {
        let mut image = blank();
        image[4096..4104].copy_from_slice(b"EFI PART");
        with_mbr_entry(&mut image, 0x0C, 256, 768);
        let report = analyze(&image, Some(1 << 20));
        assert_eq!(report.declared_bytes, Some(4 << 20));
        assert_eq!(report.truncated, Some(true));
    }

    #[test]
    fn une_position_de_secours_aberrante_ne_passe_pas_pour_une_taille() {
        for alternate in [u64::MAX, u64::MAX / 2] {
            let mut image = blank();
            with_gpt_alternate(&mut image, alternate);
            let report = analyze(&image, Some(HEAD_LEN as u64));
            assert_eq!(report.truncated, Some(true), "{alternate}");
            assert_eq!(report.declared_bytes, Some(u64::MAX), "{alternate}");
        }
    }
}
