use crate::bytes::{put_u16, put_u32, put_u64};
use schemars::JsonSchema;
use serde::Serialize;

use super::exfat_tree;
use super::{ALIGNMENT, FormatError};
use crate::block::BlockDevice;

pub(super) const BOOT_REGION: u64 = 12;
const CHECKSUM_SECTOR: usize = 11;
const MAX_CLUSTERS: u64 = (1 << 32) - 11;
pub(super) const FIRST_CLUSTER: u32 = 2;
const REVISION: u16 = 0x0100;

/// Recommended up-case table of the exFAT specification, compressed, as Windows writes it. Its published
/// checksum `E619D30D` is checked by a test.
pub(super) const UPCASE: &[u8; 5836] = include_bytes!("exfat_upcase.bin");

/// Volume shape, computed without touching the drive. Offsets and lengths in volume sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Geometry {
    pub cluster_bytes: u32,
    pub clusters: u32,
    pub fat_offset: u32,
    pub fat_length: u32,
    pub heap_offset: u32,
}

/// Windows default exFAT cluster sizes.
fn cluster_bytes_for(volume_bytes: u64) -> u32 {
    const MIB: u64 = 1 << 20;
    match volume_bytes {
        n if n <= 256 * MIB => 4096,
        n if n <= 32 * 1024 * MIB => 32_768,
        _ => 131_072,
    }
}

/// # Errors
/// Sector size outside 512 to 4096 bytes, no room for the bitmap, up-case table and root, or a volume beyond
/// the boot sector fields.
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
    // FAT and heap start on 1 MiB boundaries, aligned with erase blocks.
    let align = (ALIGNMENT / block64).max(1);
    let fat_offset = align.max(2 * BOOT_REGION);
    // Sized on the whole volume, the FAT always covers the remaining clusters.
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
    if u64::from(geometry.root_cluster() + 1 - FIRST_CLUSTER) >= clusters {
        return Err(FormatError::TooSmall(format!(
            "{volume_bytes} bytes: too small for exFAT"
        )));
    }
    Ok(geometry)
}

impl Geometry {
    pub(super) fn bitmap_bytes(&self) -> u32 {
        self.clusters.div_ceil(8)
    }

    pub(super) fn clusters_for(&self, bytes: u64) -> u64 {
        bytes.div_ceil(u64::from(self.cluster_bytes)).max(1)
    }

    pub(super) fn bitmap_cluster(&self) -> u32 {
        FIRST_CLUSTER
    }

    /// The bitmap fits in the volume: the conversion cannot fail.
    pub(super) fn upcase_cluster(&self) -> u32 {
        self.bitmap_cluster()
            + u32::try_from(self.clusters_for(u64::from(self.bitmap_bytes()))).unwrap_or(0)
    }

    pub(super) fn root_cluster(&self) -> u32 {
        self.upcase_cluster() + u32::try_from(self.clusters_for(UPCASE.len() as u64)).unwrap_or(0)
    }

    pub(super) fn cluster_offset(&self, first_lba: u64, block: u32, cluster: u32) -> u64 {
        (first_lba + u64::from(self.heap_offset)) * u64::from(block)
            + u64::from(cluster - FIRST_CLUSTER) * u64::from(self.cluster_bytes)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Volume<'a> {
    pub first_lba: u64,
    pub sectors: u64,
    /// Already checked by [`super::normalize_label`]: at most 11 UTF-16 units.
    pub label: Option<&'a str>,
    pub volume_id: u32,
}

/// Boot region and up-case table checksum: rotate right, then add the byte.
pub(super) fn checksum32(bytes: &[u8], skip: &[usize]) -> u32 {
    bytes
        .iter()
        .enumerate()
        .filter(|(index, _)| !skip.contains(index))
        .fold(0u32, |sum, (_, &byte)| {
            sum.rotate_right(1).wrapping_add(u32::from(byte))
        })
}

/// Entry set checksum and name hash: the same rotation on 16 bits.
pub(super) fn checksum16(bytes: &[u8], skip: &[usize]) -> u16 {
    bytes
        .iter()
        .enumerate()
        .filter(|(index, _)| !skip.contains(index))
        .fold(0u16, |sum, (_, &byte)| {
            sum.rotate_right(1).wrapping_add(u16::from(byte))
        })
}

/// One value per UTF-16 unit. `FFFF` then a count marks unchanged units; the final `FFFF` maps `FFFF` itself.
pub(super) fn upcase_map() -> Vec<u16> {
    let units: Vec<u16> = UPCASE
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let mut map: Vec<u16> = Vec::with_capacity(0x1_0000);
    let mut index = 0;
    while index < units.len() && map.len() < 0x1_0000 {
        if units[index] == 0xFFFF && index + 1 < units.len() {
            for _ in 0..units[index + 1] {
                map.push(map.len() as u16);
            }
            index += 2;
        } else {
            map.push(units[index]);
            index += 1;
        }
    }
    while map.len() < 0x1_0000 {
        map.push(map.len() as u16);
    }
    map
}

/// `used`: clusters in use.
pub(super) fn boot_region(
    block: u32,
    geo: &Geometry,
    volume: &Volume<'_>,
    root_cluster: u32,
    used: u32,
) -> Vec<u8> {
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
    put_u32(s, 96, root_cluster);
    put_u32(s, 100, volume.volume_id);
    put_u16(s, 104, REVISION);
    s[108] = block.trailing_zeros() as u8;
    s[109] = (geo.cluster_bytes / block).trailing_zeros() as u8;
    s[110] = 1;
    s[111] = 0x80;
    s[112] = u8::try_from(u64::from(used) * 100 / u64::from(geo.clusters)).unwrap_or(0xFF);
    // No boot code: HLT everywhere, as the specification asks.
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

/// Writes the empty volume; a refused shape writes nothing. Boot regions go last, so an interrupted write
/// never looks like a valid volume. Flushing is the caller's job.
pub fn write(dev: &mut dyn BlockDevice, volume: &Volume<'_>) -> Result<Geometry, FormatError> {
    let layout = exfat_tree::layout(volume, dev.block_size(), &[], &[])?;
    layout.write_structures(dev)?;
    layout.finish(dev, volume)?;
    Ok(*layout.geometry())
}

pub fn verify(dev: &dyn BlockDevice, volume: &Volume<'_>) -> Result<(), FormatError> {
    exfat_tree::layout(volume, dev.block_size(), &[], &[])?.verify_structures(dev, volume)
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
    fn la_table_de_majuscules_est_celle_de_la_specification() {
        assert_eq!(checksum32(UPCASE, &[]), 0xE619_D30D);
        let map = upcase_map();
        assert_eq!(map.len(), 0x1_0000);
        assert_eq!(map[usize::from(b'a')], u16::from(b'A'));
        assert_eq!(map[0xE9], 0xC9);
        assert_eq!(map[0x3C9], 0x3A9);
        assert_eq!(map[usize::from(b'A')], u16::from(b'A'));
        assert_eq!(map[0xFFFF], 0xFFFF);
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
}
