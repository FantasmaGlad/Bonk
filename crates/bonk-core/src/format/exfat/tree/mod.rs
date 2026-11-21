use std::collections::BTreeMap;
use std::io::Read;

use crate::block::BlockDevice;
use crate::format::exfat::{
    BOOT_REGION, FIRST_CLUSTER, Geometry, UPCASE, Volume, boot_region, geometry, upcase_map,
};
use crate::format::tree::{
    Children, NewFile, Placed, check_room, collect_children, place_files, verify_contiguous,
    write_contiguous,
};
use crate::format::{FormatError, zero_range};
use crate::partition::read_range;

use crate::format::exfat::END_OF_CHAIN;

mod entries;
mod plan;
#[cfg(test)]
mod tests;

pub(super) use entries::{entries_for, entry_set, name_units};
use plan::*;

const ENTRY_BITMAP: u8 = 0x81;
const ENTRY_UPCASE: u8 = 0x82;
const ATTR_ARCHIVE: u16 = 0x20;
const ALLOCATION_POSSIBLE: u8 = 0x01;
const MAX_NAME_UNITS: usize = 255;
/// 1 January 1980: nothing depends on the clock, two writes give the same bytes.
const FIXED_TIMESTAMP: u32 = 0x0021_0000;
const MEDIA: u32 = 0xFFFF_FFF8;

/// Computed before the first write; files and non-root directories are contiguous, without a FAT chain.
#[derive(Debug)]
pub struct Layout {
    geometry: Geometry,
    first_lba: u64,
    block: u32,
    root: Placed,
    directories: Vec<(Placed, Vec<u8>)>,
    files: Vec<Placed>,
    sizes: Vec<u64>,
    used_clusters: u32,
}

fn narrow(value: u64) -> Result<u32, FormatError> {
    u32::try_from(value).map_err(|_| FormatError::TooLarge(format!("{value}: too large for exFAT")))
}

type Names = BTreeMap<Vec<String>, Vec<Vec<u16>>>;

/// # Errors
/// Volume too small, refused name, names equal once upper-cased, or directory over 256 MiB.
pub fn layout(
    volume: &Volume<'_>,
    block: u32,
    files: &[NewFile],
    extra_directories: &[Vec<String>],
) -> Result<Layout, FormatError> {
    let geo = geometry(volume.sectors, block)?;
    let upcase = upcase_map();
    let children: Children = collect_children(files, extra_directories, u64::MAX, "exFAT")?;
    let names = name_units_by_directory(&children, &upcase)?;
    let mut next = u64::from(geo.root_cluster());
    let placed_dirs = place_directories(&names, &geo, volume.label.is_some(), &mut next)?;
    let cluster_bytes = u64::from(geo.cluster_bytes);
    let placed_files = place_files(files, cluster_bytes, &mut next, narrow)?;
    let used = next - u64::from(FIRST_CLUSTER);
    check_room(used, u64::from(geo.clusters), cluster_bytes)?;
    let tree = Tree {
        geo: &geo,
        label: volume.label,
        upcase: &upcase,
        children: &children,
        files,
        placed_dirs: &placed_dirs,
        placed_files: &placed_files,
    };
    let mut directories: Vec<(Placed, Vec<u8>)> = names
        .iter()
        .map(|(path, units_of)| (placed_dirs[path], tree.directory(path, units_of)))
        .collect();
    // Root first: only it is chained in the FAT.
    directories.sort_by_key(|(placed, _)| placed.first_cluster);
    Ok(Layout {
        geometry: geo,
        first_lba: volume.first_lba,
        block,
        root: placed_dirs[&Vec::new()],
        directories,
        files: placed_files,
        sizes: files.iter().map(|file| file.size).collect(),
        used_clusters: narrow(used)?,
    })
}

impl Layout {
    #[must_use]
    pub fn geometry(&self) -> &Geometry {
        &self.geometry
    }

    #[must_use]
    pub fn data_bytes(&self) -> u64 {
        self.sizes.iter().sum()
    }

    fn cluster_offset(&self, cluster: u32) -> u64 {
        self.geometry
            .cluster_offset(self.first_lba, self.block, cluster)
    }

    fn sector_offset(&self, sector: u64) -> u64 {
        (self.first_lba + sector) * u64::from(self.block)
    }

    /// Entries 0 and 1, then the chains of the bitmap, up-case table and root. Contiguous files and
    /// directories are not in it.
    fn fat_head(&self) -> Vec<u8> {
        let geo = &self.geometry;
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
        chain(
            geo.bitmap_cluster(),
            geo.upcase_cluster() - geo.bitmap_cluster(),
        );
        chain(
            geo.upcase_cluster(),
            geo.root_cluster() - geo.upcase_cluster(),
        );
        chain(self.root.first_cluster, self.root.clusters);
        let mut bytes: Vec<u8> = entries.iter().flat_map(|e| e.to_le_bytes()).collect();
        bytes.resize(bytes.len().next_multiple_of(self.block as usize), 0);
        bytes
    }

    fn bitmap(&self) -> Vec<u8> {
        let mut bytes = vec![0u8; self.geometry.bitmap_bytes() as usize];
        for index in 0..self.used_clusters as usize {
            bytes[index / 8] |= 1 << (index % 8);
        }
        bytes
    }

    fn upcase_bytes(&self) -> Vec<u8> {
        UPCASE.to_vec()
    }

    fn metadata(&self) -> Vec<(&'static str, u64, Vec<u8>)> {
        let geo = &self.geometry;
        let mut parts = vec![
            (
                "FAT",
                self.sector_offset(u64::from(geo.fat_offset)),
                self.fat_head(),
            ),
            (
                "allocation bitmap",
                self.cluster_offset(geo.bitmap_cluster()),
                self.bitmap(),
            ),
            (
                "up-case table",
                self.cluster_offset(geo.upcase_cluster()),
                self.upcase_bytes(),
            ),
        ];
        for (placed, bytes) in &self.directories {
            parts.push((
                "directory",
                self.cluster_offset(placed.first_cluster),
                bytes.clone(),
            ));
        }
        parts
    }

    /// Everything but the boot regions, so an interrupted write never looks like a valid volume.
    ///
    /// # Errors
    /// Write failure.
    pub fn write_structures(&self, dev: &mut dyn BlockDevice) -> Result<(), FormatError> {
        let start = self.sector_offset(0);
        let end = self.cluster_offset(self.root.first_cluster)
            + u64::from(self.root.clusters) * u64::from(self.geometry.cluster_bytes);
        zero_range(dev, start, end - start)?;
        for (_, offset, mut bytes) in self.metadata() {
            bytes.resize(bytes.len().next_multiple_of(self.block as usize), 0);
            dev.write_at(offset, &bytes)?;
        }
        Ok(())
    }

    fn file_offset(&self, placed: Placed) -> u64 {
        if placed.clusters == 0 {
            0
        } else {
            self.cluster_offset(placed.first_cluster)
        }
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
        let offset = self.file_offset(self.files[index]);
        write_contiguous(dev, offset, self.sizes[index], reader, advance)
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
        let offset = self.file_offset(self.files[index]);
        let what = format!("file number {index}");
        verify_contiguous(dev, offset, self.sizes[index], expected, &what, advance)
    }

    fn boot(&self, volume: &Volume<'_>) -> Vec<u8> {
        boot_region(
            self.block,
            &self.geometry,
            volume,
            self.root.first_cluster,
            self.used_clusters,
        )
    }

    /// Boot regions last, backup first.
    ///
    /// # Errors
    /// Write failure.
    pub fn finish(
        &self,
        dev: &mut dyn BlockDevice,
        volume: &Volume<'_>,
    ) -> Result<(), FormatError> {
        let region = self.boot(volume);
        for first in [BOOT_REGION, 0] {
            dev.write_at(self.sector_offset(first), &region)?;
        }
        Ok(())
    }

    /// # Errors
    /// Read failure, or mismatch.
    pub fn verify_structures(
        &self,
        dev: &dyn BlockDevice,
        volume: &Volume<'_>,
    ) -> Result<(), FormatError> {
        let region = self.boot(volume);
        let mut parts = vec![
            ("main boot region", self.sector_offset(0), region.clone()),
            (
                "backup boot region",
                self.sector_offset(BOOT_REGION),
                region,
            ),
        ];
        parts.extend(self.metadata());
        for (what, offset, bytes) in parts {
            if read_range(dev, offset, bytes.len() as u64)? != bytes {
                return Err(FormatError::Mismatch(format!(
                    "exFAT {what} read back differently"
                )));
            }
        }
        Ok(())
    }
}
