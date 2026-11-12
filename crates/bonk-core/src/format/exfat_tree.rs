use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use super::exfat::{
    BOOT_REGION, FIRST_CLUSTER, Geometry, UPCASE, Volume, boot_region, checksum16, checksum32,
    geometry, upcase_map,
};
use super::tree::{
    ChildKind, Children, NewFile, collect_children, verify_contiguous, write_contiguous,
};
use super::{FormatError, zero_range};
use crate::block::BlockDevice;
use crate::bytes::{put_u16, put_u32, put_u64};
use crate::partition::read_range;

use super::exfat::{
    ATTR_DIRECTORY, END_OF_CHAIN, ENTRY, ENTRY_FILE, ENTRY_LABEL, ENTRY_NAME, ENTRY_STREAM,
    MAX_DIRECTORY_BYTES, NAME_UNITS, NO_FAT_CHAIN,
};

const ENTRY_BITMAP: u8 = 0x81;
const ENTRY_UPCASE: u8 = 0x82;
const ATTR_ARCHIVE: u16 = 0x20;
const ALLOCATION_POSSIBLE: u8 = 0x01;
const MAX_NAME_UNITS: usize = 255;
/// 1 January 1980: nothing depends on the clock, two writes give the same bytes.
const FIXED_TIMESTAMP: u32 = 0x0021_0000;
const MEDIA: u32 = 0xFFFF_FFF8;

#[derive(Debug, Clone, Copy)]
pub(super) struct Placed {
    pub(super) first_cluster: u32,
    pub(super) clusters: u32,
}

/// Where every byte goes, computed before the first write. Each file and non-root directory is contiguous,
/// without a FAT chain.
#[derive(Debug)]
pub struct Layout {
    geometry: Geometry,
    first_lba: u64,
    block: u32,
    root: Placed,
    /// Root first.
    directories: Vec<(Placed, Vec<u8>)>,
    files: Vec<Placed>,
    sizes: Vec<u64>,
    used_clusters: u32,
}

pub(super) fn name_units(name: &str) -> Result<Vec<u16>, FormatError> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let forbidden = |c: char| c < ' ' || "\"*/:<>?\\|".contains(c);
    if units.is_empty() || units.len() > MAX_NAME_UNITS || name.chars().any(forbidden) {
        return Err(FormatError::Name(format!("{name}: name refused by exFAT")));
    }
    Ok(units)
}

pub(super) fn entries_for(units: &[u16]) -> usize {
    2 + units.len().div_ceil(NAME_UNITS)
}

pub(super) fn entry_set(
    units: &[u16],
    upcase: &[u16],
    directory: bool,
    placed: Placed,
    data_length: u64,
) -> Vec<u8> {
    let count = entries_for(units);
    let mut set = vec![0u8; count * ENTRY];
    set[0] = ENTRY_FILE;
    set[1] = (count - 1) as u8;
    put_u16(
        &mut set,
        4,
        if directory {
            ATTR_DIRECTORY
        } else {
            ATTR_ARCHIVE
        },
    );
    for at in [8, 12, 16] {
        put_u32(&mut set, at, FIXED_TIMESTAMP);
    }
    let stream = &mut set[ENTRY..2 * ENTRY];
    stream[0] = ENTRY_STREAM;
    stream[1] = if placed.clusters == 0 {
        ALLOCATION_POSSIBLE
    } else {
        ALLOCATION_POSSIBLE | NO_FAT_CHAIN
    };
    stream[3] = units.len() as u8;
    let upcased: Vec<u8> = units
        .iter()
        .flat_map(|unit| upcase[usize::from(*unit)].to_le_bytes())
        .collect();
    put_u16(stream, 4, checksum16(&upcased, &[]));
    put_u64(stream, 8, data_length);
    put_u32(stream, 20, placed.first_cluster);
    put_u64(stream, 24, data_length);
    for (index, piece) in units.chunks(NAME_UNITS).enumerate() {
        let entry = &mut set[(2 + index) * ENTRY..(3 + index) * ENTRY];
        entry[0] = ENTRY_NAME;
        for (position, unit) in piece.iter().enumerate() {
            put_u16(entry, 2 + position * 2, *unit);
        }
    }
    let sum = checksum16(&set, &[2, 3]);
    put_u16(&mut set, 2, sum);
    set
}

/// Entries the root must start with.
fn system_entries(geo: &Geometry, label: Option<&str>) -> Vec<u8> {
    let mut bytes = vec![0u8; if label.is_some() { 3 } else { 2 } * ENTRY];
    let bitmap = &mut bytes[..ENTRY];
    bitmap[0] = ENTRY_BITMAP;
    put_u32(bitmap, 20, geo.bitmap_cluster());
    put_u64(bitmap, 24, u64::from(geo.bitmap_bytes()));
    let upcase = &mut bytes[ENTRY..2 * ENTRY];
    upcase[0] = ENTRY_UPCASE;
    put_u32(upcase, 4, checksum32(UPCASE, &[]));
    put_u32(upcase, 20, geo.upcase_cluster());
    put_u64(upcase, 24, UPCASE.len() as u64);
    if let Some(label) = label {
        let entry = &mut bytes[2 * ENTRY..];
        entry[0] = ENTRY_LABEL;
        let units: Vec<u16> = label.encode_utf16().take(11).collect();
        entry[1] = units.len() as u8;
        for (i, unit) in units.iter().enumerate() {
            put_u16(entry, 2 + i * 2, *unit);
        }
    }
    bytes
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
    let mut placed_files = Vec::with_capacity(files.len());
    for file in files {
        let clusters = file.size.div_ceil(u64::from(geo.cluster_bytes));
        placed_files.push(Placed {
            first_cluster: if clusters == 0 { 0 } else { narrow(next)? },
            clusters: narrow(clusters)?,
        });
        next += clusters;
    }
    let used = next - u64::from(FIRST_CLUSTER);
    if used > u64::from(geo.clusters) {
        return Err(FormatError::TooSmall(format!(
            "{} bytes to store: the drive only has {} data bytes",
            used * u64::from(geo.cluster_bytes),
            u64::from(geo.clusters) * u64::from(geo.cluster_bytes)
        )));
    }
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

fn name_units_by_directory(children: &Children, upcase: &[u16]) -> Result<Names, FormatError> {
    let mut names = Names::new();
    for (path, list) in children {
        let mut seen = BTreeSet::new();
        let mut units_of = Vec::with_capacity(list.len());
        for child in list {
            let units = name_units(&child.name)?;
            let key: Vec<u16> = units.iter().map(|u| upcase[usize::from(*u)]).collect();
            if !seen.insert(key) {
                return Err(FormatError::Name(format!(
                    "{}: two names exFAT cannot tell apart",
                    child.name
                )));
            }
            units_of.push(units);
        }
        names.insert(path.clone(), units_of);
    }
    Ok(names)
}

fn place_directories(
    names: &Names,
    geo: &Geometry,
    labelled: bool,
    next: &mut u64,
) -> Result<BTreeMap<Vec<String>, Placed>, FormatError> {
    let mut placed = BTreeMap::new();
    for (path, units_of) in names {
        let mut entries: usize = units_of.iter().map(|u| entries_for(u)).sum();
        if path.is_empty() {
            entries += if labelled { 3 } else { 2 };
        }
        let bytes = (entries * ENTRY) as u64;
        if bytes > MAX_DIRECTORY_BYTES {
            return Err(FormatError::TooLarge(format!(
                "/{}: directory larger than exFAT allows",
                path.join("/")
            )));
        }
        let clusters = geo.clusters_for(bytes);
        placed.insert(
            path.clone(),
            Placed {
                first_cluster: narrow(*next)?,
                clusters: narrow(clusters)?,
            },
        );
        *next += clusters;
    }
    Ok(placed)
}

struct Tree<'a> {
    geo: &'a Geometry,
    label: Option<&'a str>,
    upcase: &'a [u16],
    children: &'a Children,
    files: &'a [NewFile],
    placed_dirs: &'a BTreeMap<Vec<String>, Placed>,
    placed_files: &'a [Placed],
}

impl Tree<'_> {
    fn directory(&self, path: &Vec<String>, units_of: &[Vec<u16>]) -> Vec<u8> {
        let cluster_bytes = u64::from(self.geo.cluster_bytes);
        let mut bytes = if path.is_empty() {
            system_entries(self.geo, self.label)
        } else {
            Vec::new()
        };
        for (child, units) in self.children[path].iter().zip(units_of) {
            bytes.extend(match &child.kind {
                ChildKind::File(index) => entry_set(
                    units,
                    self.upcase,
                    false,
                    self.placed_files[*index],
                    self.files[*index].size,
                ),
                ChildKind::Directory(sub) => {
                    let dir = self.placed_dirs[sub];
                    entry_set(
                        units,
                        self.upcase,
                        true,
                        dir,
                        u64::from(dir.clusters) * cluster_bytes,
                    )
                }
            });
        }
        let placed = self.placed_dirs[path];
        bytes.resize((u64::from(placed.clusters) * cluster_bytes) as usize, 0);
        bytes
    }
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::block::MemoryDevice;

    fn volume() -> Volume<'static> {
        Volume {
            first_lba: 2048,
            sectors: (64 << 20) / 512,
            label: Some("CLE"),
            volume_id: 7,
        }
    }

    fn file(path: &str, size: u64) -> NewFile {
        NewFile {
            path: path.split('/').map(str::to_owned).collect(),
            size,
        }
    }

    #[test]
    fn deux_noms_egaux_en_majuscules_sont_refuses() {
        let v = volume();
        let refused = layout(&v, 512, &[file("été.iso", 1), file("ÉTÉ.ISO", 1)], &[]);
        assert!(matches!(refused, Err(FormatError::Name(_))));
        assert!(layout(&v, 512, &[file("a:b", 1)], &[]).is_err());
        assert!(layout(&v, 512, &[file("été.iso", 1), file("ete.iso", 1)], &[]).is_ok());
    }

    /// Checksum and name hash recomputed by hand from the specification.
    #[test]
    fn jeu_d_entrees_selon_la_specification() {
        let upcase = upcase_map();
        let units: Vec<u16> = "Debian-13.iso".encode_utf16().collect();
        let placed = Placed {
            first_cluster: 9,
            clusters: 3,
        };
        let set = entry_set(&units, &upcase, false, placed, 10_000);
        assert_eq!(set.len(), 3 * ENTRY);
        assert_eq!((set[0], set[1], set[32], set[64]), (0x85, 2, 0xC0, 0xC1));
        assert_eq!(set[33], ALLOCATION_POSSIBLE | NO_FAT_CHAIN);
        let mut sum: u16 = 0;
        for (i, byte) in set.iter().enumerate() {
            if i == 2 || i == 3 {
                continue;
            }
            sum = (if sum & 1 == 1 { 0x8000 } else { 0 }) + (sum >> 1) + u16::from(*byte);
        }
        assert_eq!(u16::from_le_bytes([set[2], set[3]]), sum);
        let upper: Vec<u8> = "DEBIAN-13.ISO"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(
            u16::from_le_bytes([set[36], set[37]]),
            checksum16(&upper, &[])
        );
    }

    #[test]
    fn copie_et_relecture_detectent_un_octet_change() {
        let content: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        let files = [
            file("isos/debian/debian.iso", content.len() as u64),
            file("vide.txt", 0),
        ];
        let v = volume();
        let plan = layout(&v, 512, &files, &[vec!["boot".to_owned()]]).unwrap();
        let mut dev = MemoryDevice::new(80 << 20, 512).unwrap();
        plan.write_structures(&mut dev).unwrap();
        let sha = plan
            .write_file(&mut dev, 0, &mut content.as_slice(), &mut |_| true)
            .unwrap();
        plan.write_file(&mut dev, 1, &mut [].as_slice(), &mut |_| true)
            .unwrap();
        plan.finish(&mut dev, &v).unwrap();
        plan.verify_structures(&dev, &v).unwrap();
        plan.verify_file(&dev, 0, &sha, &mut |_| true).unwrap();
        let at = plan.cluster_offset(plan.files[0].first_cluster) as usize + 50_000;
        let mut bytes = dev.as_bytes().to_vec();
        bytes[at] ^= 1;
        let bad = MemoryDevice::from_bytes(bytes, 512).unwrap();
        assert!(matches!(
            plan.verify_file(&bad, 0, &sha, &mut |_| true),
            Err(FormatError::Mismatch(_))
        ));
    }

    #[test]
    fn un_volume_trop_petit_pour_les_fichiers_est_refuse() {
        let v = volume();
        assert!(matches!(
            layout(&v, 512, &[file("gros.iso", 1 << 30)], &[]),
            Err(FormatError::TooSmall(_))
        ));
    }

    #[test]
    fn un_fichier_de_plus_de_4_gio_est_accepte() {
        let v = Volume {
            sectors: (8u64 << 30) / 512,
            ..volume()
        };
        let plan = layout(&v, 512, &[file("windows.iso", 5 << 30)], &[]).unwrap();
        assert_eq!(plan.data_bytes(), 5 << 30);
    }
}
