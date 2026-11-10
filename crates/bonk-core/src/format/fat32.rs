use crate::bytes::{put_u16, put_u32};
use schemars::JsonSchema;
use serde::Serialize;

use super::{ALIGNMENT, FormatError, zero_range};
use crate::block::BlockDevice;
use crate::partition::read_range;

/// Minimum reserved sectors (specification).
const RESERVED: u64 = 32;
pub(super) const FATS: u64 = 2;
pub(super) const ROOT_CLUSTER: u32 = 2;
const FSINFO_SECTOR: u64 = 1;
const BACKUP_BOOT_SECTOR: u64 = 6;
/// Below this, a compliant driver reads the volume as FAT16, from the cluster count alone.
const MIN_CLUSTERS: u64 = 65_525;
const MAX_CLUSTERS: u64 = 0x0FFF_FFF5;
const MEDIA: u8 = 0xF8;
const NO_LABEL: &[u8; 11] = b"NO NAME    ";

/// Volume shape, computed without touching the drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Geometry {
    pub cluster_bytes: u32,
    pub clusters: u64,
    pub reserved_sectors: u64,
    pub fat_sectors: u64,
}

/// Cluster size from the specification table (`DskTableFAT32`).
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
/// [`FormatError::TooSmall`] under 65,525 clusters, [`FormatError::TooLarge`] beyond what FAT32 addresses.
pub fn geometry(sectors: u64, block: u32) -> Result<Geometry, FormatError> {
    if !block.is_power_of_two() || !(512..=4096).contains(&block) {
        return Err(FormatError::Unsupported(format!(
            "FAT32 needs sectors of 512 to 4096 bytes, not {block}"
        )));
    }
    let block64 = u64::from(block);
    let volume_bytes = sectors.saturating_mul(block64);
    if u32::try_from(sectors).is_err() {
        return Err(FormatError::TooLarge(format!(
            "FAT32 cannot address {volume_bytes} bytes with sectors of {block} bytes"
        )));
    }
    let cluster_bytes = cluster_bytes_for(volume_bytes).max(block);
    let per_cluster = u64::from(cluster_bytes / block);
    // Sized on the whole volume, the FAT always covers the remaining clusters: an exact fit oscillates, and a
    // FAT a few entries short would corrupt the volume.
    let fat_sectors = ((sectors / per_cluster + 2) * 4).div_ceil(block64);
    // Data area on a 1 MiB boundary, aligned with erase blocks.
    let start = RESERVED + FATS * fat_sectors;
    let aligned = (start * block64).next_multiple_of(ALIGNMENT) / block64;
    let reserved_sectors = RESERVED + (aligned - start);
    let clusters = sectors.saturating_sub(aligned) / per_cluster;
    if clusters < MIN_CLUSTERS {
        return Err(FormatError::TooSmall(format!(
            "{volume_bytes} bytes: too small for FAT32 ({clusters} clusters, at least {MIN_CLUSTERS})"
        )));
    }
    if clusters > MAX_CLUSTERS {
        return Err(FormatError::TooLarge(format!(
            "{volume_bytes} bytes: too large for FAT32"
        )));
    }
    Ok(Geometry {
        cluster_bytes,
        clusters,
        reserved_sectors,
        fat_sectors,
    })
}

/// FAT32 fields are fixed-width: a value that does not fit is refused, never truncated.
pub(super) fn narrow<T: TryFrom<u64>>(value: u64) -> Result<T, FormatError> {
    T::try_from(value).map_err(|_| FormatError::TooLarge(format!("{value}: too large for FAT32")))
}

/// Space-padded; already checked by [`super::normalize_label`].
pub(super) fn label_field(label: Option<&str>) -> [u8; 11] {
    let mut field = *NO_LABEL;
    if let Some(label) = label {
        field = [b' '; 11];
        for (slot, byte) in field.iter_mut().zip(label.bytes()) {
            *slot = byte;
        }
    }
    field
}

#[derive(Debug, Clone, Copy)]
pub struct Volume<'a> {
    pub first_lba: u64,
    pub sectors: u64,
    pub label: Option<&'a str>,
    pub volume_id: u32,
    /// [`crate::boot::VBR`] around the BPB, [`crate::boot::STAGE2`] in the reserved area.
    pub bios_boot: bool,
}

fn check_bios_boot(block: u32, volume: &Volume<'_>) -> Result<(), FormatError> {
    use crate::boot::SECTOR;
    if volume.bios_boot && block != SECTOR {
        return Err(FormatError::Unsupported(format!(
            "BIOS boot code needs sectors of {SECTOR} bytes, not {block}"
        )));
    }
    if volume.bios_boot && u32::try_from(volume.first_lba).is_err() {
        return Err(FormatError::Unsupported(
            "BIOS boot code finds its volume through a 32-bit start sector".to_owned(),
        ));
    }
    Ok(())
}

const _: () = assert!(
    crate::boot::STAGE2_SECTOR as u64
        + crate::boot::STAGE2.len() as u64 / crate::boot::SECTOR as u64
        <= RESERVED
);

fn boot_sector(block: u32, geo: &Geometry, volume: &Volume<'_>) -> Result<Vec<u8>, FormatError> {
    let mut s = vec![0u8; block as usize];
    if volume.bios_boot {
        s.copy_from_slice(crate::boot::VBR);
    }
    s[..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    s[3..11].copy_from_slice(b"MSWIN4.1");
    put_u16(&mut s, 11, narrow(u64::from(block))?);
    s[13] = narrow(u64::from(geo.cluster_bytes / block))?;
    put_u16(&mut s, 14, narrow(geo.reserved_sectors)?);
    s[16] = narrow(FATS)?;
    s[21] = MEDIA;
    put_u16(&mut s, 24, 63);
    put_u16(&mut s, 26, 255);
    // Hidden sectors only serve boot code, checked above: beyond 32 bits the field stays 0.
    put_u32(&mut s, 28, u32::try_from(volume.first_lba).unwrap_or(0));
    put_u32(&mut s, 32, narrow(volume.sectors)?);
    put_u32(&mut s, 36, narrow(geo.fat_sectors)?);
    put_u32(&mut s, 44, ROOT_CLUSTER);
    put_u16(&mut s, 48, narrow(FSINFO_SECTOR)?);
    put_u16(&mut s, 50, narrow(BACKUP_BOOT_SECTOR)?);
    s[64] = 0x80;
    s[66] = 0x29;
    put_u32(&mut s, 67, volume.volume_id);
    s[71..82].copy_from_slice(&label_field(volume.label));
    s[82..90].copy_from_slice(b"FAT32   ");
    s[510] = 0x55;
    s[511] = 0xAA;
    Ok(s)
}

fn fsinfo_sector(block: u32, free_clusters: u64, next_free: u32) -> Result<Vec<u8>, FormatError> {
    let mut s = vec![0u8; block as usize];
    put_u32(&mut s, 0, 0x4161_5252);
    put_u32(&mut s, 484, 0x6141_7272);
    put_u32(&mut s, 488, narrow(free_clusters)?);
    put_u32(&mut s, 492, next_free);
    s[510] = 0x55;
    s[511] = 0xAA;
    Ok(s)
}

/// The reserved entries 0 and 1, then the root directory's end of chain.
pub(super) const FAT_HEAD: [u32; 3] = [0x0FFF_FF00 | MEDIA as u32, 0x0FFF_FFFF, 0x0FFF_FFFF];

fn first_fat_sector(block: u32) -> Vec<u8> {
    let mut s = vec![0u8; block as usize];
    for (index, entry) in FAT_HEAD.into_iter().enumerate() {
        put_u32(&mut s, index * 4, entry);
    }
    s
}

/// Writes the empty volume; a refused shape writes nothing. The boot sector goes last, so an interrupted
/// write never looks like a valid filesystem. Flushing is the caller's job.
pub fn write(dev: &mut dyn BlockDevice, volume: &Volume<'_>) -> Result<Geometry, FormatError> {
    let block = dev.block_size();
    let block64 = u64::from(block);
    let geo = geometry(volume.sectors, block)?;
    check_bios_boot(block, volume)?;
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
    // Only the root cluster is in use.
    write_boot(dev, volume, &geo, geo.clusters - 1, ROOT_CLUSTER + 1)?;
    Ok(geo)
}

pub(super) fn write_boot(
    dev: &mut dyn BlockDevice,
    volume: &Volume<'_>,
    geo: &Geometry,
    free_clusters: u64,
    next_free: u32,
) -> Result<(), FormatError> {
    let block = dev.block_size();
    check_bios_boot(block, volume)?;
    let at = |sector: u64| (volume.first_lba + sector) * u64::from(block);
    if volume.bios_boot {
        dev.write_at(
            at(u64::from(crate::boot::STAGE2_SECTOR)),
            crate::boot::STAGE2,
        )?;
    }
    let boot = boot_sector(block, geo, volume)?;
    let fsinfo = fsinfo_sector(block, free_clusters, next_free)?;
    let mut third = vec![0u8; block as usize];
    third[510] = 0x55;
    third[511] = 0xAA;
    for start in [BACKUP_BOOT_SECTOR, 0] {
        dev.write_at(at(start + 1), &fsinfo)?;
        dev.write_at(at(start + 2), &third)?;
        dev.write_at(at(start), &boot)?;
    }
    Ok(())
}

pub fn verify(dev: &dyn BlockDevice, volume: &Volume<'_>) -> Result<(), FormatError> {
    let block = dev.block_size();
    let geo = verify_boot(dev, volume)?;
    let fat = first_fat_sector(block);
    for sector in [geo.reserved_sectors, geo.reserved_sectors + geo.fat_sectors] {
        compare_sector(dev, volume, sector, &fat)?;
    }
    Ok(())
}

pub(super) fn verify_boot(
    dev: &dyn BlockDevice,
    volume: &Volume<'_>,
) -> Result<Geometry, FormatError> {
    let block = dev.block_size();
    let geo = geometry(volume.sectors, block)?;
    check_bios_boot(block, volume)?;
    let boot = boot_sector(block, &geo, volume)?;
    for sector in [0, BACKUP_BOOT_SECTOR] {
        compare_sector(dev, volume, sector, &boot)?;
    }
    if volume.bios_boot {
        compare_sector(
            dev,
            volume,
            u64::from(crate::boot::STAGE2_SECTOR),
            crate::boot::STAGE2,
        )?;
    }
    Ok(geo)
}

fn compare_sector(
    dev: &dyn BlockDevice,
    volume: &Volume<'_>,
    sector: u64,
    expected: &[u8],
) -> Result<(), FormatError> {
    let block = u64::from(dev.block_size());
    let found = read_range(
        dev,
        (volume.first_lba + sector) * block,
        expected.len() as u64,
    )?;
    if found == expected {
        Ok(())
    } else {
        Err(FormatError::Mismatch(format!(
            "volume sector {sector}: the read-back differs from what was written"
        )))
    }
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
            bios_boot: false,
        }
    }

    #[test]
    fn le_code_bios_refuse_un_debut_au_dela_de_32_bits() -> Result<(), FormatError> {
        let loin = Volume {
            first_lba: 1 << 32,
            bios_boot: true,
            ..volume(GIB / 512)
        };
        assert!(matches!(
            check_bios_boot(512, &loin),
            Err(FormatError::Unsupported(_))
        ));
        let sans_code = Volume {
            bios_boot: false,
            ..loin
        };
        assert!(check_bios_boot(512, &sans_code).is_ok());
        let geo = geometry(sans_code.sectors, 512)?;
        let boot = boot_sector(512, &geo, &sans_code)?;
        assert_eq!(u32_at(&boot, 28), 0);
        Ok(())
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
            // Beyond the 32 GiB the Windows tool limits FAT32 to.
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
        assert!(matches!(
            geometry(1 << 20, 1 << 17),
            Err(FormatError::Unsupported(_))
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
    fn volume_amorcable_porte_le_code_bios_et_le_second_etage()
    -> Result<(), Box<dyn std::error::Error>> {
        let sectors = 64 * MIB / 512;
        let mut dev = MemoryDevice::new(65 * MIB, 512)?;
        let vol = Volume {
            bios_boot: true,
            ..volume(sectors)
        };
        write(&mut dev, &vol)?;
        verify(&dev, &vol)?;
        let base = usize::try_from(vol.first_lba * 512)?;
        let bytes = dev.as_bytes();
        let boot = &bytes[base..base + 512];
        assert_eq!(&boot[..3], &crate::boot::VBR[..3]);
        assert_eq!(&boot[90..], &crate::boot::VBR[90..]);
        assert_eq!(&boot[82..90], b"FAT32   ");
        assert_eq!(u32_at(boot, 28), 2048);
        assert_eq!(&bytes[base + 6 * 512..base + 7 * 512], boot);
        let stage2 = base + crate::boot::STAGE2_SECTOR as usize * 512;
        assert_eq!(&bytes[stage2..stage2 + 4096], &crate::boot::STAGE2[..]);

        let mut altered = bytes.to_vec();
        altered[stage2 + 100] ^= 0xFF;
        let altered = MemoryDevice::from_bytes(altered, 512)?;
        assert!(matches!(
            verify(&altered, &vol),
            Err(FormatError::Mismatch(_))
        ));
        Ok(())
    }

    #[test]
    fn code_bios_refuse_hors_secteurs_de_512_octets() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(513 * MIB, 4096)?;
        let vol = Volume {
            first_lba: 256,
            bios_boot: true,
            ..volume(512 * MIB / 4096)
        };
        assert!(matches!(
            write(&mut dev, &vol),
            Err(FormatError::Unsupported(_))
        ));
        assert!(dev.as_bytes().iter().all(|b| *b == 0));
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
