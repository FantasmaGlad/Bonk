//! Écriture d'un système de fichiers FAT32 vide (spécification Microsoft « FAT32 File System », fatgen103).
//!
//! Écriture maison plutôt que `fatfs` : un volume vide ne demande que quelques structures fixes, et
//! `fatfs` ne connaît pas les accès alignés qu'impose `BlockDevice`.

use schemars::JsonSchema;
use serde::Serialize;

use super::{ALIGNMENT, FormatError, zero_range};
use crate::block::BlockDevice;
use crate::partition::read_range;

/// Secteurs réservés avant les tables, au minimum (valeur de la spécification).
const RESERVED: u64 = 32;
const FATS: u64 = 2;
const ROOT_CLUSTER: u32 = 2;
const FSINFO_SECTOR: u64 = 1;
const BACKUP_BOOT_SECTOR: u64 = 6;
/// En deçà, un pilote conforme lit le volume comme du FAT16, d'après la seule valeur du nombre de clusters.
const MIN_CLUSTERS: u64 = 65_525;
const MAX_CLUSTERS: u64 = 0x0FFF_FFF5;
const MEDIA: u8 = 0xF8;
const NO_LABEL: &[u8; 11] = b"NO NAME    ";

/// Forme d'un volume, calculée sans toucher au support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Geometry {
    pub cluster_bytes: u32,
    pub clusters: u64,
    pub reserved_sectors: u64,
    pub fat_sectors: u64,
}

/// Taille de cluster de la table de la spécification (`DskTableFAT32`), selon la taille du volume.
fn cluster_bytes_for(volume_bytes: u64) -> u32 {
    const SECTOR: u64 = 512;
    match volume_bytes / SECTOR {
        n if n <= 532_480 => 512,
        n if n <= 16_777_216 => 4096,
        n if n <= 33_554_432 => 8192,
        n if n <= 67_108_864 => 16_384,
        _ => 32_768,
    }
}

/// # Errors
/// [`FormatError::TooSmall`] sous 65 525 clusters, [`FormatError::TooLarge`] au-delà de ce que FAT32 adresse.
pub fn geometry(sectors: u64, block: u32) -> Result<Geometry, FormatError> {
    let block64 = u64::from(block);
    let volume_bytes = sectors.saturating_mul(block64);
    if u32::try_from(sectors).is_err() {
        return Err(FormatError::TooLarge(format!(
            "FAT32 n'adresse pas {volume_bytes} octets avec des secteurs de {block} octets"
        )));
    }
    let cluster_bytes = cluster_bytes_for(volume_bytes).max(block);
    let per_cluster = u64::from(cluster_bytes / block);
    // Une table comptée sur tout le volume couvre toujours les clusters restants : calculer au plus juste
    // oscille, et une table trop courte de quelques entrées corromprait le volume. La marge est infime.
    let fat_sectors = ((sectors / per_cluster + 2) * 4).div_ceil(block64);
    // La zone de données commence sur 1 Mio : les clusters tombent alors sur les blocs d'effacement.
    let start = RESERVED + FATS * fat_sectors;
    let aligned = (start * block64).next_multiple_of(ALIGNMENT) / block64;
    let reserved_sectors = RESERVED + (aligned - start);
    let clusters = sectors.saturating_sub(aligned) / per_cluster;
    if clusters < MIN_CLUSTERS {
        return Err(FormatError::TooSmall(format!(
            "{volume_bytes} octets : trop petit pour FAT32 ({clusters} clusters, {MIN_CLUSTERS} au minimum)"
        )));
    }
    if clusters > MAX_CLUSTERS {
        return Err(FormatError::TooLarge(format!(
            "{volume_bytes} octets : trop grand pour FAT32"
        )));
    }
    Ok(Geometry {
        cluster_bytes,
        clusters,
        reserved_sectors,
        fat_sectors,
    })
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

/// Valeurs bornées par [`geometry`] : la conversion ne peut pas échouer.
fn narrow<T: TryFrom<u64> + Default>(value: u64) -> T {
    T::try_from(value).unwrap_or_default()
}

/// Étiquette complétée d'espaces ; déjà validée par [`super::normalize_label`].
fn label_field(label: Option<&str>) -> [u8; 11] {
    let mut field = *NO_LABEL;
    if let Some(label) = label {
        field = [b' '; 11];
        for (slot, byte) in field.iter_mut().zip(label.bytes()) {
            *slot = byte;
        }
    }
    field
}

/// Ce qu'on écrit sur un volume : ses paramètres, pour écrire puis pour relire.
#[derive(Debug, Clone, Copy)]
pub struct Volume<'a> {
    pub first_lba: u64,
    pub sectors: u64,
    pub label: Option<&'a str>,
    pub volume_id: u32,
}

fn boot_sector(block: u32, geo: &Geometry, volume: &Volume<'_>) -> Vec<u8> {
    let mut s = vec![0u8; block as usize];
    s[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    s[3..11].copy_from_slice(b"MSWIN4.1");
    put_u16(&mut s, 11, narrow(u64::from(block)));
    s[13] = narrow(u64::from(geo.cluster_bytes / block));
    put_u16(&mut s, 14, narrow(geo.reserved_sectors));
    s[16] = narrow(FATS);
    s[21] = MEDIA;
    put_u16(&mut s, 24, 63);
    put_u16(&mut s, 26, 255);
    put_u32(&mut s, 28, narrow(volume.first_lba));
    put_u32(&mut s, 32, narrow(volume.sectors));
    put_u32(&mut s, 36, narrow(geo.fat_sectors));
    put_u32(&mut s, 44, ROOT_CLUSTER);
    put_u16(&mut s, 48, narrow(FSINFO_SECTOR));
    put_u16(&mut s, 50, narrow(BACKUP_BOOT_SECTOR));
    s[64] = 0x80;
    s[66] = 0x29;
    put_u32(&mut s, 67, volume.volume_id);
    s[71..82].copy_from_slice(&label_field(volume.label));
    s[82..90].copy_from_slice(b"FAT32   ");
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

fn fsinfo_sector(block: u32, geo: &Geometry) -> Vec<u8> {
    let mut s = vec![0u8; block as usize];
    put_u32(&mut s, 0, 0x4161_5252);
    put_u32(&mut s, 484, 0x6141_7272);
    // Le cluster racine est le seul occupé.
    put_u32(&mut s, 488, narrow(geo.clusters - 1));
    put_u32(&mut s, 492, ROOT_CLUSTER + 1);
    s[510] = 0x55;
    s[511] = 0xAA;
    s
}

fn first_fat_sector(block: u32) -> Vec<u8> {
    let mut s = vec![0u8; block as usize];
    put_u32(&mut s, 0, 0x0FFF_FF00 | u32::from(MEDIA));
    put_u32(&mut s, 4, 0x0FFF_FFFF);
    put_u32(&mut s, 8, 0x0FFF_FFFF);
    s
}

/// Écrit le volume vide ; une forme refusée n'écrit rien. Le secteur de démarrage vient en dernier :
/// une coupure laisse un volume que personne ne prend pour un système de fichiers valide. Le vidage
/// reste à l'appelant.
pub fn write(dev: &mut dyn BlockDevice, volume: &Volume<'_>) -> Result<Geometry, FormatError> {
    let block = dev.block_size();
    let block64 = u64::from(block);
    let geo = geometry(volume.sectors, block)?;
    let at = |sector: u64| (volume.first_lba + sector) * block64;

    let data_start = geo.reserved_sectors + FATS * geo.fat_sectors;
    let root = data_start * block64;
    zero_range(
        dev,
        volume.first_lba * block64,
        root + u64::from(geo.cluster_bytes),
    )?;
    for fat in 0..FATS {
        let sector = geo.reserved_sectors + fat * geo.fat_sectors;
        dev.write_at(at(sector), &first_fat_sector(block))?;
    }
    if let Some(label) = volume.label {
        let mut entry = vec![0u8; block as usize];
        entry[..11].copy_from_slice(&label_field(Some(label)));
        entry[11] = 0x08;
        dev.write_at(at(data_start), &entry)?;
    }
    let boot = boot_sector(block, &geo, volume);
    let fsinfo = fsinfo_sector(block, &geo);
    let mut third = vec![0u8; block as usize];
    third[510] = 0x55;
    third[511] = 0xAA;
    for start in [BACKUP_BOOT_SECTOR, 0] {
        dev.write_at(at(start + 1), &fsinfo)?;
        dev.write_at(at(start + 2), &third)?;
        dev.write_at(at(start), &boot)?;
    }
    Ok(geo)
}

/// Relit ce qui définit le volume : secteur de démarrage et sa copie, début des deux tables.
pub fn verify(dev: &dyn BlockDevice, volume: &Volume<'_>) -> Result<(), FormatError> {
    let block = dev.block_size();
    let block64 = u64::from(block);
    let geo = geometry(volume.sectors, block)?;
    let boot = boot_sector(block, &geo, volume);
    let fat = first_fat_sector(block);
    let expected: [(u64, &[u8]); 4] = [
        (0, &boot),
        (BACKUP_BOOT_SECTOR, &boot),
        (geo.reserved_sectors, &fat),
        (geo.reserved_sectors + geo.fat_sectors, &fat),
    ];
    for (sector, bytes) in expected {
        let found = read_range(dev, (volume.first_lba + sector) * block64, block64)?;
        if found != bytes {
            return Err(FormatError::Mismatch(format!(
                "secteur {sector} du volume : la relecture diffère de ce qui a été écrit"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockError, MemoryDevice};

    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;

    fn volume(sectors: u64) -> Volume<'static> {
        Volume {
            first_lba: 2048,
            sectors,
            label: Some("CLE"),
            volume_id: 0x1234_5678,
        }
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    #[test]
    fn taille_de_cluster_selon_le_volume() -> Result<(), FormatError> {
        for (bytes, cluster) in [
            (4 * GIB, 4096),
            (10 * GIB, 8192),
            (20 * GIB, 16_384),
            (64 * GIB, 32_768),
            // Au-delà des 32 Gio auxquels l'outil de Windows limite FAT32.
            (512 * GIB, 32_768),
        ] {
            assert_eq!(
                geometry(bytes / 512, 512)?.cluster_bytes,
                cluster,
                "{bytes}"
            );
        }
        Ok(())
    }

    #[test]
    fn la_zone_de_donnees_commence_sur_un_mio() -> Result<(), FormatError> {
        for sectors in [131_072u64, 8 * GIB / 512, 64 * GIB / 512] {
            let g = geometry(sectors, 512)?;
            let start = (g.reserved_sectors + 2 * g.fat_sectors) * 512;
            assert_eq!(start % MIB, 0, "{sectors}");
            assert!(g.reserved_sectors >= RESERVED);
            // La table couvre tous les clusters.
            assert!(g.fat_sectors * 512 >= (g.clusters + 2) * 4);
        }
        Ok(())
    }

    #[test]
    fn refuse_trop_petit_et_trop_grand_sans_ecrire() -> Result<(), BlockError> {
        assert!(matches!(geometry(1000, 512), Err(FormatError::TooSmall(_))));
        assert!(matches!(
            geometry(1 << 33, 512),
            Err(FormatError::TooLarge(_))
        ));
        let mut dev = MemoryDevice::new(MIB, 512)?;
        assert!(write(&mut dev, &volume(100)).is_err());
        assert!(dev.as_bytes().iter().all(|b| *b == 0));
        Ok(())
    }

    #[test]
    fn structures_ecrites_et_relues() -> Result<(), Box<dyn std::error::Error>> {
        let sectors = 64 * MIB / 512;
        let mut dev = MemoryDevice::new(65 * MIB, 512)?;
        let vol = volume(sectors);
        let geo = write(&mut dev, &vol)?;
        verify(&dev, &vol)?;
        let base = usize::try_from(vol.first_lba * 512)?;
        let bytes = dev.as_bytes();
        let boot = &bytes[base..base + 512];
        assert_eq!(&boot[82..90], b"FAT32   ");
        assert_eq!(&boot[71..82], b"CLE        ");
        assert_eq!((boot[510], boot[511]), (0x55, 0xAA));
        assert_eq!(u32_at(boot, 28), 2048);
        assert_eq!(&bytes[base + 6 * 512..base + 7 * 512], boot);
        let fsinfo = &bytes[base + 512..base + 1024];
        assert_eq!(u32_at(fsinfo, 0), 0x4161_5252);
        assert_eq!(u64::from(u32_at(fsinfo, 488)), geo.clusters - 1);
        let fat = base + usize::try_from(geo.reserved_sectors * 512)?;
        assert_eq!(u32_at(bytes, fat + 8), 0x0FFF_FFFF);
        let root = fat + usize::try_from(2 * geo.fat_sectors * 512)?;
        assert_eq!(&bytes[root..root + 11], b"CLE        ");
        assert_eq!(bytes[root + 11], 0x08);
        Ok(())
    }

    #[test]
    fn sans_etiquette_pas_d_entree_de_volume() -> Result<(), Box<dyn std::error::Error>> {
        let sectors = 64 * MIB / 512;
        let mut dev = MemoryDevice::new(65 * MIB, 512)?;
        let vol = Volume {
            label: None,
            ..volume(sectors)
        };
        let geo = write(&mut dev, &vol)?;
        let base = usize::try_from(vol.first_lba * 512)?;
        let bytes = dev.as_bytes();
        assert_eq!(&bytes[base + 71..base + 82], NO_LABEL);
        let root = base + usize::try_from((geo.reserved_sectors + 2 * geo.fat_sectors) * 512)?;
        assert_eq!(bytes[root], 0);
        Ok(())
    }

    #[test]
    fn un_support_qui_rend_autre_chose_est_detecte() -> Result<(), Box<dyn std::error::Error>> {
        let sectors = 64 * MIB / 512;
        let mut dev = MemoryDevice::new(65 * MIB, 512)?;
        let vol = volume(sectors);
        write(&mut dev, &vol)?;
        let mut altered = dev.as_bytes().to_vec();
        altered[usize::try_from(vol.first_lba * 512)?] ^= 0xFF;
        let dev = MemoryDevice::from_bytes(altered, 512)?;
        assert!(matches!(verify(&dev, &vol), Err(FormatError::Mismatch(_))));
        Ok(())
    }

    #[test]
    fn secteurs_de_4096_octets() -> Result<(), Box<dyn std::error::Error>> {
        let sectors = 512 * MIB / 4096;
        let mut dev = MemoryDevice::new(513 * MIB, 4096)?;
        let vol = Volume {
            first_lba: 256,
            ..volume(sectors)
        };
        let geo = write(&mut dev, &vol)?;
        assert_eq!(geo.cluster_bytes, 4096);
        verify(&dev, &vol)?;
        Ok(())
    }
}
