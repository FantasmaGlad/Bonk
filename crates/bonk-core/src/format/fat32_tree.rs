use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;

use super::fat32::{self, FAT_HEAD, FATS, Geometry, ROOT_CLUSTER, Volume, narrow, write_boot};
use super::tree::{
    ChildKind, Children, NewFile, collect_children, verify_contiguous, write_contiguous,
};
use super::{FormatError, zero_range};
use crate::block::BlockDevice;
use crate::partition::read_range;

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

#[derive(Debug, Clone, Copy)]
struct Placed {
    first_cluster: u32,
    clusters: u32,
}

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

fn is_plain_short(name: &str) -> bool {
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let good = |part: &str, max: usize| {
        part.len() <= max
            && part
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
    };
    !base.is_empty() && good(base, 8) && good(ext, 3)
}

fn short_chars(part: &str, max: usize) -> String {
    part.chars()
        .filter_map(|c| {
            let c = c.to_ascii_uppercase();
            (c.is_ascii_alphanumeric() || c == '_' || c == '-').then_some(c)
        })
        .take(max)
        .collect()
}

fn pack_short(base: &str, ext: &str) -> [u8; 11] {
    let mut field = [b' '; 11];
    for (slot, byte) in field[..8].iter_mut().zip(base.bytes()) {
        *slot = byte;
    }
    for (slot, byte) in field[8..].iter_mut().zip(ext.bytes()) {
        *slot = byte;
    }
    field
}

/// The upper-cased name if valid, else `BASE~N.EXT`. The flag says a long entry must keep the exact name.
fn short_name(name: &str, taken: &mut BTreeSet<[u8; 11]>) -> Result<([u8; 11], bool), FormatError> {
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let upper = name.to_ascii_uppercase();
    if is_plain_short(&upper) {
        let (upper_base, upper_ext) = upper.rsplit_once('.').unwrap_or((&upper, ""));
        let field = pack_short(upper_base, upper_ext);
        if taken.insert(field) {
            return Ok((field, upper != name));
        }
    }
    let (clean_base, clean_ext) = (short_chars(base, 8), short_chars(ext, 3));
    for number in 1..1_000_000u32 {
        let suffix = format!("~{number}");
        let keep = 8 - suffix.len();
        let stem: String = clean_base.chars().take(keep).collect();
        let field = pack_short(&format!("{stem}{suffix}"), &clean_ext);
        if taken.insert(field) {
            return Ok((field, true));
        }
    }
    Err(FormatError::Name(format!("{name}: no free short name")))
}

fn short_checksum(short: &[u8; 11]) -> u8 {
    short
        .iter()
        .fold(0u8, |sum, byte| sum.rotate_right(1).wrapping_add(*byte))
}

/// Last to first, as the format requires.
fn long_entries(name: &str, short: &[u8; 11]) -> Result<Vec<[u8; ENTRY]>, FormatError> {
    let mut units: Vec<u16> = name.encode_utf16().collect();
    if units.len() > MAX_NAME_UNITS || name.chars().any(|c| "\"*/:<>?\\|".contains(c) || c < ' ') {
        return Err(FormatError::Name(format!("{name}: name refused by FAT32")));
    }
    if !units.len().is_multiple_of(LONG_NAME_UNITS) {
        units.push(0);
    }
    let count = units.len().div_ceil(LONG_NAME_UNITS);
    let checksum = short_checksum(short);
    let mut entries = Vec::with_capacity(count);
    for index in (0..count).rev() {
        let mut entry = [0u8; ENTRY];
        let order = u8::try_from(index + 1).map_err(|_| FormatError::Name(name.to_owned()))?;
        entry[0] = if index + 1 == count {
            order | 0x40
        } else {
            order
        };
        entry[11] = ATTR_LONG_NAME;
        entry[13] = checksum;
        let piece = &units[index * LONG_NAME_UNITS..units.len().min((index + 1) * LONG_NAME_UNITS)];
        let slots = (1..11)
            .step_by(2)
            .chain((14..26).step_by(2))
            .chain((28..32).step_by(2));
        for (position, slot) in slots.enumerate() {
            let unit = piece.get(position).copied().unwrap_or(0xFFFF);
            entry[slot..slot + 2].copy_from_slice(&unit.to_le_bytes());
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn short_entry(short: &[u8; 11], attributes: u8, cluster: u32, size: u32) -> [u8; ENTRY] {
    let mut entry = [0u8; ENTRY];
    entry[..11].copy_from_slice(short);
    entry[11] = attributes;
    for at in [16, 18, 24] {
        entry[at..at + 2].copy_from_slice(&FIXED_DATE.to_le_bytes());
    }
    entry[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    entry[26..28].copy_from_slice(&((cluster & 0xFFFF) as u16).to_le_bytes());
    entry[28..32].copy_from_slice(&size.to_le_bytes());
    entry
}

const DOT: [u8; 11] = *b".          ";
const DOT_DOT: [u8; 11] = *b"..         ";

type Encoded = BTreeMap<Vec<String>, Vec<Row>>;
type PlacedDirs = BTreeMap<Vec<String>, (u32, u32)>;

fn encode_names(children: &Children) -> Result<Encoded, FormatError> {
    let mut encoded = Encoded::new();
    for (path, list) in children {
        let mut taken = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut rows = Vec::with_capacity(list.len());
        for (position, child) in list.iter().enumerate() {
            if !seen.insert(child.name.to_lowercase()) {
                return Err(FormatError::Name(format!(
                    "{}: two names FAT32 cannot tell apart",
                    child.name
                )));
            }
            let (short, needs_long) = short_name(&child.name, &mut taken)?;
            let long = if needs_long {
                long_entries(&child.name, &short)?
            } else {
                Vec::new()
            };
            rows.push((long, short, position));
        }
        encoded.insert(path.clone(), rows);
    }
    Ok(encoded)
}

fn directory_bytes(
    label: Option<&str>,
    files: &[NewFile],
    children: &Children,
    encoded: &Encoded,
    placed_dirs: &PlacedDirs,
    placed_files: &[Placed],
) -> Result<Vec<(u32, u32, Vec<u8>)>, FormatError> {
    let mut directories = Vec::with_capacity(encoded.len());
    for (path, rows) in encoded {
        let (first, clusters) = placed_dirs[path];
        let mut bytes = Vec::new();
        if path.is_empty() {
            if let Some(label) = label {
                bytes.extend_from_slice(&short_entry(
                    &fat32::label_field(Some(label)),
                    ATTR_LABEL,
                    0,
                    0,
                ));
            }
        } else {
            let parent_first = match placed_dirs[&path[..path.len() - 1]].0 {
                // `..` pointing to the root is written 0.
                ROOT_CLUSTER => 0,
                other => other,
            };
            bytes.extend_from_slice(&short_entry(&DOT, ATTR_DIRECTORY, first, 0));
            bytes.extend_from_slice(&short_entry(&DOT_DOT, ATTR_DIRECTORY, parent_first, 0));
        }
        let list = &children[path];
        for (long, short, position) in rows {
            for entry in long {
                bytes.extend_from_slice(entry);
            }
            let row = match &list[*position].kind {
                ChildKind::File(index) => short_entry(
                    short,
                    ATTR_ARCHIVE,
                    placed_files[*index].first_cluster,
                    narrow(files[*index].size)?,
                ),
                ChildKind::Directory(child) => {
                    short_entry(short, ATTR_DIRECTORY, placed_dirs[child].0, 0)
                }
            };
            bytes.extend_from_slice(&row);
        }
        directories.push((first, clusters, bytes));
    }
    Ok(directories)
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
    let mut placed_files = Vec::with_capacity(files.len());
    for file in files {
        let clusters = file.size.div_ceil(cluster);
        placed_files.push(Placed {
            first_cluster: if clusters == 0 { 0 } else { narrow(next)? },
            clusters: narrow(clusters)?,
        });
        next += clusters;
    }
    let used = next - u64::from(ROOT_CLUSTER);
    if used > geometry.clusters {
        return Err(FormatError::TooSmall(format!(
            "{} bytes to store: the drive only has {} data bytes",
            used * cluster,
            geometry.clusters * cluster
        )));
    }
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
            bios_boot: false,
        }
    }

    fn file(path: &str, size: u64) -> NewFile {
        NewFile {
            path: path.split('/').map(str::to_owned).collect(),
            size,
        }
    }

    #[test]
    fn un_nom_court_valide_reste_tel_quel_les_autres_recoivent_un_nom_unique() {
        let mut taken = BTreeSet::new();
        assert!(!short_name("BOOT.WIM", &mut taken).unwrap().1);
        let (a, long) = short_name("bootmgr-x.efi", &mut taken).unwrap();
        assert!(long);
        assert_eq!(&a, b"BOOTMG~1EFI");
        let (b, _) = short_name("bootmgr-y.efi", &mut taken).unwrap();
        assert_eq!(&b, b"BOOTMG~2EFI");
    }

    /// The BIOS second stage looks for `BOOTMGR`; Windows ISOs name it in lower case.
    #[test]
    fn un_nom_court_en_minuscules_garde_son_nom_en_majuscules() {
        let mut taken = BTreeSet::new();
        let (short, long) = short_name("bootmgr", &mut taken).unwrap();
        assert_eq!(&short, b"BOOTMGR    ");
        assert!(long);
        assert_eq!(
            &short_name("setup.exe", &mut taken).unwrap().0,
            b"SETUP   EXE"
        );
        let (again, _) = short_name("BootMgr", &mut taken).unwrap();
        assert_eq!(&again, b"BOOTMG~1   ");
    }

    /// Specification values: short name checksum, entry order and padding.
    #[test]
    fn les_entrees_longues_suivent_le_format() {
        let short = *b"BOOTMG~1EFI";
        let entries = long_entries("bootmgr_long_name.efi", &short).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0][0], 0x42);
        assert_eq!(entries[1][0], 0x01);
        assert!(entries.iter().all(|e| e[11] == ATTR_LONG_NAME));
        assert_eq!(entries[1][1], b'b');
        assert_eq!(entries[0][13], short_checksum(&short));
        assert!(long_entries("a:b", &short).is_err());
    }

    #[test]
    fn les_refus_passent_avant_toute_ecriture() {
        let tiny = |f: &[NewFile]| layout(&volume(), 512, f, &[]);
        assert!(matches!(
            tiny(&[file("a", 1 << 32)]),
            Err(FormatError::TooLarge(_))
        ));
        assert!(matches!(
            tiny(&[file("a", 1 << 31)]),
            Err(FormatError::TooSmall(_))
        ));
        assert!(matches!(
            tiny(&[file("A.txt", 1), file("a.TXT", 1)]),
            Err(FormatError::Name(_))
        ));
    }

    #[test]
    fn copie_et_relecture_detectent_un_octet_change() {
        let content: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        let files = [
            file("sources/install.swm", content.len() as u64),
            file("vide.txt", 0),
        ];
        let v = volume();
        let plan = layout(&v, 512, &files, &[vec!["boot".to_owned()]]).unwrap();
        let mut dev = MemoryDevice::new(80 << 20, 512).unwrap();
        plan.write_structures(&mut dev).unwrap();
        let mut copied = 0;
        let sha = plan
            .write_file(&mut dev, 0, &mut content.as_slice(), &mut |n| {
                copied += n;
                true
            })
            .unwrap();
        assert_eq!(copied, content.len() as u64);
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
    fn un_fichier_plus_court_qu_annonce_est_une_erreur() {
        let v = volume();
        let plan = layout(&v, 512, &[file("a", 1000)], &[]).unwrap();
        let mut dev = MemoryDevice::new(80 << 20, 512).unwrap();
        plan.write_structures(&mut dev).unwrap();
        assert!(matches!(
            plan.write_file(&mut dev, 0, &mut [0u8; 10].as_slice(), &mut |_| true),
            Err(FormatError::Source(_))
        ));
    }
}
