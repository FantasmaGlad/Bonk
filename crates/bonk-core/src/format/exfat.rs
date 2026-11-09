use crate::bytes::{put_u16, put_u32, put_u64};
use schemars::JsonSchema;
use serde::Serialize;

use super::{ALIGNMENT, FormatError, zero_range};
use crate::block::BlockDevice;
use crate::partition::read_range;

/// Secteurs d'une région de démarrage ; la copie de secours suit la principale.
const BOOT_REGION: u64 = 12;
const CHECKSUM_SECTOR: usize = 11;
const MAX_CLUSTERS: u64 = (1 << 32) - 11;
const FIRST_CLUSTER: u32 = 2;
const END_OF_CHAIN: u32 = 0xFFFF_FFFF;
const MEDIA: u32 = 0xFFFF_FFF8;
const REVISION: u16 = 0x0100;
const ENTRY: usize = 32;
const ENTRY_BITMAP: u8 = 0x81;
const ENTRY_UPCASE: u8 = 0x82;
const ENTRY_LABEL: u8 = 0x83;

/// Forme d'un volume, calculée sans toucher au support. Offsets et longueurs en secteurs du volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Geometry {
    pub cluster_bytes: u32,
    pub clusters: u32,
    pub fat_offset: u32,
    pub fat_length: u32,
    pub heap_offset: u32,
}

/// Tailles de cluster par défaut de Windows pour exFAT, selon la taille du volume.
fn cluster_bytes_for(volume_bytes: u64) -> u32 {
    const MIB: u64 = 1 << 20;
    match volume_bytes {
        n if n <= 256 * MIB => 4096,
        n if n <= 32 * 1024 * MIB => 32_768,
        _ => 131_072,
    }
}

/// # Errors
/// [`FormatError::Unsupported`] pour un secteur hors de 512 à 4 096 octets, [`FormatError::TooSmall`] sans
/// place pour la table d'allocation, la table de majuscules et la racine, [`FormatError::TooLarge`] au-delà
/// de ce qu'adressent les champs du secteur de démarrage.
pub fn geometry(sectors: u64, block: u32) -> Result<Geometry, FormatError> {
    if !block.is_power_of_two() || !(512..=4096).contains(&block) {
        return Err(FormatError::Unsupported(format!(
            "exFAT needs sectors of 512 to 4096 bytes, not {block}"
        )));
    }
    let block64 = u64::from(block);
    let volume_bytes = sectors.saturating_mul(block64);
    let cluster_bytes = cluster_bytes_for(volume_bytes).max(block);
    let per_cluster = u64::from(cluster_bytes / block);
    // Table et tas commencent chacun sur 1 Mio : les clusters tombent sur les blocs d'effacement.
    let align = (ALIGNMENT / block64).max(1);
    let fat_offset = align.max(2 * BOOT_REGION);
    // Comptée sur tout le volume, la table couvre toujours les clusters restants.
    let fat_length = ((sectors / per_cluster + 2) * 4).div_ceil(block64);
    let heap_offset = (fat_offset + fat_length).next_multiple_of(align);
    let clusters = (sectors.saturating_sub(heap_offset) / per_cluster).min(MAX_CLUSTERS);
    let too_large = || FormatError::TooLarge(format!("{volume_bytes} bytes: too large for exFAT"));
    let geometry = Geometry {
        cluster_bytes,
        clusters: u32::try_from(clusters).map_err(|_| too_large())?,
        fat_offset: u32::try_from(fat_offset).map_err(|_| too_large())?,
        fat_length: u32::try_from(fat_length).map_err(|_| too_large())?,
        heap_offset: u32::try_from(heap_offset).map_err(|_| too_large())?,
    };
    if u64::from(geometry.metadata_clusters()) >= clusters {
        return Err(FormatError::TooSmall(format!(
            "{volume_bytes} bytes: too small for exFAT"
        )));
    }
    Ok(geometry)
}

impl Geometry {
    fn bitmap_bytes(&self) -> u32 {
        self.clusters.div_ceil(8)
    }

    fn clusters_for(&self, bytes: u32) -> u32 {
        bytes.div_ceil(self.cluster_bytes).max(1)
    }

    fn bitmap_cluster(&self) -> u32 {
        FIRST_CLUSTER
    }

    fn upcase_cluster(&self) -> u32 {
        self.bitmap_cluster() + self.clusters_for(self.bitmap_bytes())
    }

    fn root_cluster(&self) -> u32 {
        self.upcase_cluster() + self.clusters_for(UPCASE_BYTES)
    }

    /// Table d'allocation, table de majuscules et un cluster de racine.
    fn metadata_clusters(&self) -> u32 {
        self.root_cluster() + 1 - FIRST_CLUSTER
    }

    /// Octet du support où commence un cluster du tas.
    fn cluster_offset(&self, first_lba: u64, block: u32, cluster: u32) -> u64 {
        (first_lba + u64::from(self.heap_offset)) * u64::from(block)
            + u64::from(cluster - FIRST_CLUSTER) * u64::from(self.cluster_bytes)
    }
}

/// Ce qu'on écrit sur un volume : ses paramètres, pour écrire puis pour relire.
#[derive(Debug, Clone, Copy)]
pub struct Volume<'a> {
    pub first_lba: u64,
    pub sectors: u64,
    /// Déjà validée par [`super::normalize_label`] : 11 unités UTF-16 au plus.
    pub label: Option<&'a str>,
    pub volume_id: u32,
}

/// Somme des régions de démarrage et de la table de majuscules : rotation à droite, puis ajout de l'octet.
fn checksum32(bytes: &[u8], skip: &[usize]) -> u32 {
    bytes
        .iter()
        .enumerate()
        .filter(|(index, _)| !skip.contains(index))
        .fold(0u32, |sum, (_, &byte)| {
            sum.rotate_right(1).wrapping_add(u32::from(byte))
        })
}

/// Majuscules des seules lettres ASCII, tout le reste identique : table complète, compressée, et la même
/// quelle que soit la version d'Unicode du compilateur. Deux noms qui ne diffèrent que par la casse d'une
/// lettre non ASCII restent distincts sur ce volume.
const UPCASE: [u16; 30] = upcase_table();
const UPCASE_BYTES: u32 = (UPCASE.len() * 2) as u32;

const fn upcase_table() -> [u16; 30] {
    let mut table = [0u16; 30];
    table[0] = 0xFFFF;
    table[1] = 0x61;
    let mut i = 0;
    while i < 26 {
        table[2 + i] = 0x41 + i as u16;
        i += 1;
    }
    table[28] = 0xFFFF;
    table[29] = (0x1_0000 - 0x7B) as u16;
    table
}

fn upcase_bytes() -> Vec<u8> {
    UPCASE.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

fn boot_region(block: u32, geo: &Geometry, volume: &Volume<'_>) -> Vec<u8> {
    let block_len = block as usize;
    let mut region = vec![0u8; block_len * BOOT_REGION as usize];
    let s = &mut region[..block_len];
    s[..3].copy_from_slice(&[0xEB, 0x76, 0x90]);
    s[3..11].copy_from_slice(b"EXFAT   ");
    put_u64(s, 72, volume.sectors);
    put_u32(s, 80, geo.fat_offset);
    put_u32(s, 84, geo.fat_length);
    put_u32(s, 88, geo.heap_offset);
    put_u32(s, 92, geo.clusters);
    put_u32(s, 96, geo.root_cluster());
    put_u32(s, 100, volume.volume_id);
    put_u16(s, 104, REVISION);
    s[108] = block.trailing_zeros() as u8;
    s[109] = (geo.cluster_bytes / block).trailing_zeros() as u8;
    s[110] = 1;
    s[111] = 0x80;
    s[112] = u8::try_from(u64::from(geo.metadata_clusters()) * 100 / u64::from(geo.clusters))
        .unwrap_or(0xFF);
    // Aucun code d'amorçage : HLT partout, comme le veut la spécification.
    s[120..510].fill(0xF4);
    s[510] = 0x55;
    s[511] = 0xAA;
    for sector in 1..=8 {
        let end = (sector + 1) * block_len;
        region[end - 4..end].copy_from_slice(&[0x00, 0x00, 0x55, 0xAA]);
    }
    let sum = checksum32(&region[..CHECKSUM_SECTOR * block_len], &[106, 107, 112]);
    for word in region[CHECKSUM_SECTOR * block_len..].as_chunks_mut::<4>().0 {
        *word = sum.to_le_bytes();
    }
    region
}

/// Début de la table : entrées 0 et 1, puis les chaînes de la table d'allocation, de la table de majuscules
/// et de la racine. Les fichiers contigus n'y figurent pas (`NoFatChain`).
fn fat_head(block: u32, geo: &Geometry) -> Vec<u8> {
    let mut entries = vec![MEDIA, END_OF_CHAIN];
    let mut chain = |first: u32, count: u32| {
        for cluster in first..first + count {
            entries.push(if cluster + 1 == first + count {
                END_OF_CHAIN
            } else {
                cluster + 1
            });
        }
    };
    chain(geo.bitmap_cluster(), geo.clusters_for(geo.bitmap_bytes()));
    chain(geo.upcase_cluster(), geo.clusters_for(UPCASE_BYTES));
    chain(geo.root_cluster(), 1);
    let mut bytes: Vec<u8> = entries.iter().flat_map(|e| e.to_le_bytes()).collect();
    bytes.resize(bytes.len().next_multiple_of(block as usize), 0);
    bytes
}

fn bitmap(geo: &Geometry, used: u32) -> Vec<u8> {
    let mut bytes = vec![0u8; geo.bitmap_bytes() as usize];
    for index in 0..used as usize {
        bytes[index / 8] |= 1 << (index % 8);
    }
    bytes
}

fn root_directory(geo: &Geometry, label: Option<&str>) -> Vec<u8> {
    let mut root = vec![0u8; geo.cluster_bytes as usize];
    let bitmap_entry = &mut root[..ENTRY];
    bitmap_entry[0] = ENTRY_BITMAP;
    put_u32(bitmap_entry, 20, geo.bitmap_cluster());
    put_u64(bitmap_entry, 24, u64::from(geo.bitmap_bytes()));
    let upcase_entry = &mut root[ENTRY..2 * ENTRY];
    upcase_entry[0] = ENTRY_UPCASE;
    put_u32(upcase_entry, 4, checksum32(&upcase_bytes(), &[]));
    put_u32(upcase_entry, 20, geo.upcase_cluster());
    put_u64(upcase_entry, 24, u64::from(UPCASE_BYTES));
    if let Some(label) = label {
        let entry = &mut root[2 * ENTRY..3 * ENTRY];
        entry[0] = ENTRY_LABEL;
        let units: Vec<u16> = label.encode_utf16().take(11).collect();
        entry[1] = units.len() as u8;
        for (i, unit) in units.iter().enumerate() {
            put_u16(entry, 2 + i * 2, *unit);
        }
    }
    root
}

/// Écrit le volume vide ; une forme refusée n'écrit rien. Les régions de démarrage viennent en dernier :
/// une coupure laisse un support que personne ne prend pour un volume valide. Le vidage reste à l'appelant.
pub fn write(dev: &mut dyn BlockDevice, volume: &Volume<'_>) -> Result<Geometry, FormatError> {
    let block = dev.block_size();
    let geo = geometry(volume.sectors, block)?;
    let start = volume.first_lba * u64::from(block);
    let metadata_end = geo.cluster_offset(volume.first_lba, block, geo.root_cluster())
        + u64::from(geo.cluster_bytes);
    zero_range(dev, start, metadata_end - start)?;
    write_metadata(dev, volume, &geo)?;
    write_boot(dev, volume, &geo)?;
    Ok(geo)
}

/// Table, table d'allocation, table de majuscules et racine.
fn write_metadata(
    dev: &mut dyn BlockDevice,
    volume: &Volume<'_>,
    geo: &Geometry,
) -> Result<(), FormatError> {
    let block = dev.block_size();
    let at = |sector: u64| (volume.first_lba + sector) * u64::from(block);
    dev.write_at(at(u64::from(geo.fat_offset)), &fat_head(block, geo))?;
    let cluster = |c| geo.cluster_offset(volume.first_lba, block, c);
    let used = geo.metadata_clusters();
    dev.write_at(
        cluster(geo.bitmap_cluster()),
        &pad(bitmap(geo, used), block),
    )?;
    dev.write_at(cluster(geo.upcase_cluster()), &pad(upcase_bytes(), block))?;
    dev.write_at(
        cluster(geo.root_cluster()),
        &root_directory(geo, volume.label),
    )?;
    Ok(())
}

fn pad(mut bytes: Vec<u8>, block: u32) -> Vec<u8> {
    bytes.resize(bytes.len().next_multiple_of(block as usize), 0);
    bytes
}

/// Les deux régions de démarrage, la copie de secours d'abord.
fn write_boot(
    dev: &mut dyn BlockDevice,
    volume: &Volume<'_>,
    geo: &Geometry,
) -> Result<(), FormatError> {
    let block = dev.block_size();
    let region = boot_region(block, geo, volume);
    for first in [BOOT_REGION, 0] {
        dev.write_at((volume.first_lba + first) * u64::from(block), &region)?;
    }
    Ok(())
}

/// Relit ce qui définit le volume vide : régions de démarrage, début de la table, table d'allocation,
/// table de majuscules et racine.
pub fn verify(dev: &dyn BlockDevice, volume: &Volume<'_>) -> Result<(), FormatError> {
    let block = dev.block_size();
    let geo = geometry(volume.sectors, block)?;
    let at = |sector: u64| (volume.first_lba + sector) * u64::from(block);
    let cluster = |c| geo.cluster_offset(volume.first_lba, block, c);
    let region = boot_region(block, &geo, volume);
    let expected: [(&str, u64, Vec<u8>); 6] = [
        ("main boot region", at(0), region.clone()),
        ("backup boot region", at(BOOT_REGION), region),
        ("FAT", at(u64::from(geo.fat_offset)), fat_head(block, &geo)),
        (
            "allocation bitmap",
            cluster(geo.bitmap_cluster()),
            bitmap(&geo, geo.metadata_clusters()),
        ),
        (
            "up-case table",
            cluster(geo.upcase_cluster()),
            upcase_bytes(),
        ),
        (
            "root directory",
            cluster(geo.root_cluster()),
            root_directory(&geo, volume.label),
        ),
    ];
    for (what, offset, bytes) in expected {
        if read_range(dev, offset, bytes.len() as u64)? != bytes {
            return Err(FormatError::Mismatch(format!(
                "exFAT {what} read back differently"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;

    const MIB: u64 = 1 << 20;
    const GIB: u64 = 1 << 30;

    fn volume(sectors: u64) -> Volume<'static> {
        Volume {
            first_lba: 2048,
            sectors,
            label: Some("Clé"),
            volume_id: 0x1234_5678,
        }
    }

    fn u32_at(bytes: &[u8], at: usize) -> u32 {
        u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
    }

    #[test]
    fn taille_de_cluster_et_alignement() -> Result<(), FormatError> {
        for (bytes, cluster) in [(64 * MIB, 4096), (8 * GIB, 32_768), (64 * GIB, 131_072)] {
            let g = geometry(bytes / 512, 512)?;
            assert_eq!(g.cluster_bytes, cluster, "{bytes}");
            assert_eq!(u64::from(g.fat_offset) * 512 % MIB, 0);
            assert_eq!(u64::from(g.heap_offset) * 512 % MIB, 0);
            assert!(u64::from(g.fat_length) * 512 >= (u64::from(g.clusters) + 2) * 4);
            assert!(
                u64::from(g.heap_offset) + u64::from(g.clusters) * u64::from(cluster / 512)
                    <= bytes / 512
            );
        }
        Ok(())
    }

    #[test]
    fn refuse_trop_petit_et_secteur_impossible_sans_ecrire()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(matches!(geometry(4096, 512), Err(FormatError::TooSmall(_))));
        assert!(matches!(
            geometry(1 << 20, 520),
            Err(FormatError::Unsupported(_))
        ));
        let mut dev = MemoryDevice::new(4 * MIB, 512)?;
        assert!(write(&mut dev, &volume(4096)).is_err());
        assert!(dev.as_bytes().iter().all(|b| *b == 0));
        Ok(())
    }

    #[test]
    fn volume_ecrit_et_relu() -> Result<(), Box<dyn std::error::Error>> {
        for block in [512u32, 4096] {
            let sectors = 64 * MIB / u64::from(block);
            let mut dev = MemoryDevice::new(66 * MIB, block)?;
            let vol = Volume {
                first_lba: MIB / u64::from(block),
                ..volume(sectors)
            };
            let geo = write(&mut dev, &vol)?;
            verify(&dev, &vol)?;
            let base = usize::try_from(vol.first_lba * u64::from(block))?;
            let bytes = dev.as_bytes();
            let boot = &bytes[base..base + 512];
            assert_eq!(&boot[3..11], b"EXFAT   ");
            assert_eq!(u32_at(boot, 96), geo.root_cluster());
            let b = block as usize;
            let region = &bytes[base..base + 12 * b];
            assert_eq!(&bytes[base + 12 * b..base + 24 * b], region);
            let sum = checksum32(&region[..11 * b], &[106, 107, 112]);
            assert_eq!(u32_at(region, 11 * b), sum);
            assert_eq!(u32_at(region, 12 * b - 4), sum);
        }
        Ok(())
    }

    #[test]
    fn un_support_qui_rend_autre_chose_est_detecte() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(66 * MIB, 512)?;
        let vol = volume(64 * MIB / 512);
        let geo = write(&mut dev, &vol)?;
        let root = usize::try_from(geo.cluster_offset(vol.first_lba, 512, geo.root_cluster()))?;
        let mut altered = dev.as_bytes().to_vec();
        altered[root + 1] ^= 0xFF;
        let dev = MemoryDevice::from_bytes(altered, 512)?;
        assert!(matches!(verify(&dev, &vol), Err(FormatError::Mismatch(_))));
        Ok(())
    }
}
