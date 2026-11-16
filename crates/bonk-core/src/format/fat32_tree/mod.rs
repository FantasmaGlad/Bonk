use std::io::Read;

use super::fat32::{self, FAT_HEAD, FATS, Geometry, ROOT_CLUSTER, Volume, narrow, write_boot};
use super::tree::{
    ChildKind, Children, NewFile, Placed, check_room, collect_children, place_files,
    verify_contiguous, write_contiguous,
};
use super::{FormatError, zero_range};
use crate::block::BlockDevice;
use crate::partition::read_range;

mod dirent;
#[cfg(test)]
mod tests;

use dirent::*;

const ENTRY: usize = 32;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_ARCHIVE: u8 = 0x20;
const ATTR_LABEL: u8 = 0x08;
const ATTR_LONG_NAME: u8 = 0x0F;
const END_OF_CHAIN: u32 = 0x0FFF_FFFF;
const FIXED_DATE: u16 = 0x0021;
const LONG_NAME_UNITS: usize = 13;
const MAX_NAME_UNITS: usize = 255;
pub const MAX_FILE_BYTES: u64 = u32::MAX as u64;

/// Long entries, short name, rank in its directory.
type Row = (Vec<[u8; ENTRY]>, [u8; 11], usize);

#[derive(Debug)]
pub struct Layout {
    geometry: Geometry,
    first_lba: u64,
    block: u64,
    files: Vec<Placed>,
    sizes: Vec<u64>,
    directories: Vec<(u32, u32, Vec<u8>)>,
    used_clusters: u32,
}

/// # Errors
/// Volume too small, file of 4 GiB or more, refused name, or duplicate.
pub fn layout(
    volume: &Volume<'_>,
    block: u32,
    files: &[NewFile],
    extra_directories: &[Vec<String>],
) -> Result<Layout, FormatError> {
    let geometry = fat32::geometry(volume.sectors, block)?;
    let cluster = u64::from(geometry.cluster_bytes);
    let children = collect_children(files, extra_directories, MAX_FILE_BYTES, "FAT32")?;
    let encoded = encode_names(&children)?;

    let mut next = u64::from(ROOT_CLUSTER);
    let mut placed_dirs = PlacedDirs::new();
    let has_label = volume.label.is_some();
    for (path, rows) in &encoded {
        let specials =
            usize::from(path.is_empty() && has_label) + if path.is_empty() { 0 } else { 2 };
        let entries: usize = rows
            .iter()
            .map(|(long, _, _)| long.len() + 1)
            .sum::<usize>()
            + specials;
        let clusters = ((entries * ENTRY) as u64).div_ceil(cluster).max(1);
        placed_dirs.insert(path.clone(), (narrow(next)?, narrow(clusters)?));
        next += clusters;
    }
    let placed_files = place_files(files, cluster, &mut next, narrow::<u32>)?;
    let used = next - u64::from(ROOT_CLUSTER);
    check_room(used, geometry.clusters, cluster)?;
    let directories = directory_bytes(
        volume.label,
        files,
        &children,
        &encoded,
        &placed_dirs,
        &placed_files,
    )?;
    Ok(Layout {
        geometry,
        first_lba: volume.first_lba,
        block: u64::from(block),
        files: placed_files,
        sizes: files.iter().map(|file| file.size).collect(),
        directories,
        used_clusters: narrow(used)?,
    })
}

impl Layout {
    /// An empty file has no cluster: nothing is read or written there.
    fn file_offset(&self, placed: Placed) -> u64 {
        if placed.clusters == 0 {
            0
        } else {
            self.cluster_offset(placed.first_cluster)
        }
    }

    fn cluster_offset(&self, cluster: u32) -> u64 {
        let data = self.geometry.reserved_sectors + FATS * self.geometry.fat_sectors;
        (self.first_lba + data) * self.block
            + u64::from(cluster - ROOT_CLUSTER) * u64::from(self.geometry.cluster_bytes)
    }

    #[must_use]
    pub fn data_bytes(&self) -> u64 {
        self.sizes.iter().sum()
    }

    fn fat_bytes(&self) -> Vec<u8> {
        let entries = self.used_clusters as usize + ROOT_CLUSTER as usize;
        let mut table = vec![0u32; entries];
        let mut chain = |first: u32, clusters: u32| {
            for cluster in first..first + clusters {
                table[cluster as usize] = if cluster + 1 == first + clusters {
                    END_OF_CHAIN
                } else {
                    cluster + 1
                };
            }
        };
        for (first, clusters, _) in &self.directories {
            chain(*first, *clusters);
        }
        for file in &self.files {
            if file.clusters > 0 {
                chain(file.first_cluster, file.clusters);
            }
        }
        table[..2].copy_from_slice(&FAT_HEAD[..2]);
        let mut bytes: Vec<u8> = table.iter().flat_map(|e| e.to_le_bytes()).collect();
        let block = self.block as usize;
        bytes.resize(bytes.len().next_multiple_of(block), 0);
        bytes
    }

    /// Tables and directories without the boot sector, so an interrupted write never looks like a valid
    /// volume.
    ///
    /// # Errors
    /// Write failure.
    pub fn write_structures(&self, dev: &mut dyn BlockDevice) -> Result<(), FormatError> {
        let geo = self.geometry;
        let at = |sector: u64| (self.first_lba + sector) * self.block;
        let fat_start = at(0);
        let fat_len = (geo.reserved_sectors + FATS * geo.fat_sectors) * self.block;
        zero_range(dev, fat_start, fat_len)?;
        let fat = self.fat_bytes();
        for copy in 0..FATS {
            dev.write_at(at(geo.reserved_sectors + copy * geo.fat_sectors), &fat)?;
        }
        for (first, clusters, bytes) in &self.directories {
            let mut image = bytes.clone();
            image.resize(*clusters as usize * geo.cluster_bytes as usize, 0);
            dev.write_at(self.cluster_offset(*first), &image)?;
        }
        Ok(())
    }

    /// Returns the SHA-256 of what was read. `advance` gets the bytes of each block and returns false to
    /// stop.
    ///
    /// # Errors
    /// Read or write failure, short file, or stop requested (`Interrupted`).
    pub fn write_file(
        &self,
        dev: &mut dyn BlockDevice,
        index: usize,
        reader: &mut dyn Read,
        advance: &mut dyn FnMut(u64) -> bool,
    ) -> Result<String, FormatError> {
        let (size, placed) = (self.sizes[index], self.files[index]);
        write_contiguous(dev, self.file_offset(placed), size, reader, advance)
    }

    /// # Errors
    /// Read failure, hash mismatch, or stop requested.
    pub fn verify_file(
        &self,
        dev: &dyn BlockDevice,
        index: usize,
        expected: &str,
        advance: &mut dyn FnMut(u64) -> bool,
    ) -> Result<(), FormatError> {
        let (size, placed) = (self.sizes[index], self.files[index]);
        verify_contiguous(
            dev,
            self.file_offset(placed),
            size,
            expected,
            &format!("file number {index}"),
            advance,
        )
    }

    /// Boot sector and FSInfo, last.
    ///
    /// # Errors
    /// Write failure.
    pub fn finish(
        &self,
        dev: &mut dyn BlockDevice,
        volume: &Volume<'_>,
    ) -> Result<(), FormatError> {
        write_boot(
            dev,
            volume,
            &self.geometry,
            self.geometry.clusters - u64::from(self.used_clusters),
            ROOT_CLUSTER + self.used_clusters,
        )
    }

    /// # Errors
    /// Read failure, or mismatch.
    pub fn verify_structures(
        &self,
        dev: &dyn BlockDevice,
        volume: &Volume<'_>,
    ) -> Result<(), FormatError> {
        fat32::verify_boot(dev, volume)?;
        let fat = self.fat_bytes();
        for copy in 0..FATS {
            let sector = self.geometry.reserved_sectors + copy * self.geometry.fat_sectors;
            let found = read_range(
                dev,
                (self.first_lba + sector) * self.block,
                fat.len() as u64,
            )?;
            if found != fat {
                return Err(FormatError::Mismatch(
                    "FAT read back differently".to_owned(),
                ));
            }
        }
        for (first, _, bytes) in &self.directories {
            let found = read_range(dev, self.cluster_offset(*first), bytes.len() as u64)?;
            if &found != bytes {
                return Err(FormatError::Mismatch(
                    "directory read back differently".to_owned(),
                ));
            }
        }
        Ok(())
    }
}
