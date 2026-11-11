use schemars::JsonSchema;
use serde::Serialize;

use super::FormatError;
use super::exfat::checksum16;
use crate::block::BlockDevice;
use crate::bytes::{u32_le, u64_le};
use crate::partition::read_range;

use super::exfat::{
    ATTR_DIRECTORY, END_OF_CHAIN, ENTRY, ENTRY_FILE, ENTRY_LABEL, ENTRY_NAME, ENTRY_STREAM,
    FIRST_CLUSTER, MAX_DIRECTORY_BYTES, NAME_UNITS, NO_FAT_CHAIN,
};

const ENTRY_END: u8 = 0x00;
const LABEL_UNITS: usize = 11;
const MAX_DEPTH: usize = 16;
const MAX_ENTRIES: usize = 100_000;

/// A file of the volume; `path` uses `/` from the root, without a leading one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileEntry {
    pub path: String,
    pub size_bytes: u64,
}

/// Reads an exFAT volume written by any system, never trusting its fields.
pub struct Reader<'a> {
    dev: &'a dyn BlockDevice,
    fat: u64,
    heap: u64,
    cluster_bytes: u64,
    clusters: u32,
    root: u32,
}

pub(super) struct Raw {
    pub(super) name: String,
    pub(super) directory: bool,
    pub(super) first_cluster: u32,
    pub(super) length: u64,
    pub(super) contiguous: bool,
}

fn invalid(what: impl Into<String>) -> FormatError {
    FormatError::Invalid(what.into())
}

fn units(bytes: &[u8]) -> impl Iterator<Item = u16> + '_ {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
}

impl<'a> Reader<'a> {
    /// `first_lba` and `sectors` in device sectors: the partition holding the volume.
    pub fn open(
        dev: &'a dyn BlockDevice,
        first_lba: u64,
        sectors: u64,
    ) -> Result<Self, FormatError> {
        let block = u64::from(dev.block_size());
        let start = first_lba
            .checked_mul(block)
            .ok_or_else(|| invalid("partition beyond the drive"))?;
        let span = sectors.saturating_mul(block);
        if start.saturating_add(span) > dev.size() || span < 512 {
            return Err(invalid("partition beyond the drive"));
        }
        let boot = read_range(dev, start, 512)?;
        if &boot[3..11] != b"EXFAT   " || boot[510..512] != [0x55, 0xAA] {
            return Err(invalid("not an exFAT volume"));
        }
        let sector_shift = boot[108];
        let cluster_shift = boot[109];
        if !(9..=12).contains(&sector_shift) || cluster_shift > 25 - sector_shift {
            return Err(invalid("exFAT sector or cluster size out of range"));
        }
        let sector = 1u64 << sector_shift;
        let volume = u64_le(&boot, 72).saturating_mul(sector);
        if volume > span {
            return Err(invalid("exFAT volume larger than its partition"));
        }
        let fats = boot[110];
        let fat_length = u64::from(u32_le(&boot, 84)) * sector;
        // TexFAT: the second FAT is the active one when the flag says so.
        let active = u64::from(fats == 2 && boot[106] & 1 == 1);
        let fat = u64::from(u32_le(&boot, 80)) * sector + active * fat_length;
        let heap = u64::from(u32_le(&boot, 88)) * sector;
        let cluster_bytes = sector << cluster_shift;
        let clusters = u32_le(&boot, 92);
        let root = u32_le(&boot, 96);
        let heap_end = heap.saturating_add(u64::from(clusters).saturating_mul(cluster_bytes));
        let fat_end = fat.saturating_add((u64::from(clusters) + 2) * 4);
        if !(1..=2).contains(&fats) || heap_end > volume || fat_end > volume {
            return Err(invalid("exFAT fields point outside the volume"));
        }
        let reader = Self {
            dev,
            fat: start + fat,
            heap: start + heap,
            cluster_bytes,
            clusters,
            root,
        };
        if !reader.valid(root) {
            return Err(invalid("exFAT root directory outside the volume"));
        }
        Ok(reader)
    }

    pub(super) fn cluster_bytes(&self) -> u64 {
        self.cluster_bytes
    }

    pub(super) fn clusters(&self) -> u32 {
        self.clusters
    }

    pub(super) fn root(&self) -> u32 {
        self.root
    }

    /// Clusters of a FAT chain, in order; bounded by the volume so a looping chain stops.
    pub(super) fn chain(&self, first: u32) -> Result<Vec<u32>, FormatError> {
        let mut chain = Vec::new();
        let mut cluster = first;
        loop {
            if !self.valid(cluster) || chain.len() as u64 > u64::from(self.clusters) {
                return Err(invalid("exFAT cluster chain broken"));
            }
            chain.push(cluster);
            match self.next(cluster)? {
                END_OF_CHAIN => return Ok(chain),
                next => cluster = next,
            }
        }
    }

    pub(super) fn valid(&self, cluster: u32) -> bool {
        cluster >= FIRST_CLUSTER && cluster - FIRST_CLUSTER < self.clusters
    }

    pub(super) fn offset(&self, cluster: u32) -> u64 {
        self.heap + u64::from(cluster - FIRST_CLUSTER) * self.cluster_bytes
    }

    fn next(&self, cluster: u32) -> Result<u32, FormatError> {
        let raw = read_range(self.dev, self.fat + u64::from(cluster) * 4, 4)?;
        Ok(u32_le(&raw, 0))
    }

    /// `length`: `None` for the root, whose size only the FAT chain gives.
    pub(super) fn read(
        &self,
        first: u32,
        length: Option<u64>,
        contiguous: bool,
    ) -> Result<Vec<u8>, FormatError> {
        if first == 0 {
            return Ok(Vec::new());
        }
        if !self.valid(first) {
            return Err(invalid("exFAT cluster outside the volume"));
        }
        if length.is_some_and(|n| n > MAX_DIRECTORY_BYTES) {
            return Err(invalid("exFAT directory too large"));
        }
        if contiguous {
            let length = length.unwrap_or(self.cluster_bytes);
            let count = length.div_ceil(self.cluster_bytes);
            if u64::from(first - FIRST_CLUSTER) + count > u64::from(self.clusters) {
                return Err(invalid("exFAT directory beyond the volume"));
            }
            return Ok(read_range(self.dev, self.offset(first), length)?);
        }
        let mut out = Vec::new();
        let mut cluster = first;
        loop {
            if out.len() as u64 + self.cluster_bytes > MAX_DIRECTORY_BYTES {
                return Err(invalid("exFAT directory too large or looping"));
            }
            out.extend(read_range(
                self.dev,
                self.offset(cluster),
                self.cluster_bytes,
            )?);
            if length.is_some_and(|n| out.len() as u64 >= n) {
                break;
            }
            match self.next(cluster)? {
                END_OF_CHAIN => break,
                next if self.valid(next) => cluster = next,
                _ => return Err(invalid("exFAT cluster chain broken")),
            }
        }
        if let Some(n) = length {
            out.truncate(usize::try_from(n).unwrap_or(usize::MAX));
        }
        Ok(out)
    }

    /// Entry sets whose checksum or shape is wrong are skipped, as systems do.
    pub(super) fn entries(directory: &[u8], label: &mut Option<String>) -> Vec<Raw> {
        let slots = directory.as_chunks::<ENTRY>().0;
        let mut found = Vec::new();
        let mut index = 0;
        while index < slots.len() {
            let kind = slots[index][0];
            if kind == ENTRY_END {
                break;
            }
            if kind == ENTRY_LABEL {
                let count = usize::from(slots[index][1]).min(LABEL_UNITS);
                let name: Vec<u16> = units(&slots[index][2..2 + count * 2]).collect();
                *label = Some(String::from_utf16_lossy(&name));
            }
            if kind != ENTRY_FILE {
                index += 1;
                continue;
            }
            let secondary = usize::from(slots[index][1]);
            let end = index + 1 + secondary;
            if secondary >= 2 && end <= slots.len() {
                found.extend(Self::entry_set(&directory[index * ENTRY..end * ENTRY]));
                index = end;
            } else {
                index += 1;
            }
        }
        found
    }

    fn entry_set(set: &[u8]) -> Option<Raw> {
        let stored = u16::from_le_bytes([set[2], set[3]]);
        if checksum16(set, &[2, 3]) != stored || set[ENTRY] != ENTRY_STREAM {
            return None;
        }
        let stream = &set[ENTRY..2 * ENTRY];
        let length = usize::from(stream[3]);
        let name: Vec<u16> = set[2 * ENTRY..]
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .take_while(|slot| slot[0] == ENTRY_NAME)
            .flat_map(|slot| units(&slot[2..2 + NAME_UNITS * 2]))
            .take(length)
            .collect();
        if name.len() < length {
            return None;
        }
        let name = String::from_utf16_lossy(&name);
        if name.is_empty() || name.contains(['/', '\\', '\0']) {
            return None;
        }
        Some(Raw {
            name,
            directory: u16::from_le_bytes([set[4], set[5]]) & ATTR_DIRECTORY != 0,
            first_cluster: u32_le(stream, 20),
            length: u64_le(stream, 24),
            contiguous: stream[1] & NO_FAT_CHAIN != 0,
        })
    }

    pub fn label(&self) -> Result<Option<String>, FormatError> {
        let mut label = None;
        Self::entries(&self.read(self.root, None, false)?, &mut label);
        Ok(label)
    }

    /// Every file, directories walked to [`MAX_DEPTH`].
    pub fn files(&self) -> Result<Vec<FileEntry>, FormatError> {
        let mut files = Vec::new();
        let mut seen = 0usize;
        let mut pending = vec![(String::new(), self.read(self.root, None, false)?, 0usize)];
        while let Some((prefix, directory, depth)) = pending.pop() {
            for raw in Self::entries(&directory, &mut None) {
                seen += 1;
                if seen > MAX_ENTRIES {
                    return Err(invalid("too many exFAT entries"));
                }
                let path = format!("{prefix}{}", raw.name);
                if !raw.directory {
                    files.push(FileEntry {
                        path,
                        size_bytes: raw.length,
                    });
                } else if depth < MAX_DEPTH {
                    let content = self.read(raw.first_cluster, Some(raw.length), raw.contiguous)?;
                    pending.push((format!("{path}/"), content, depth + 1));
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;
    use crate::format::NewFile;
    use crate::format::exfat::{self, Volume};
    use crate::format::exfat_tree;

    fn file(path: &str, size: u64) -> NewFile {
        NewFile {
            path: path.split('/').map(str::to_owned).collect(),
            size,
        }
    }

    const MIB: u64 = 1 << 20;

    fn volume(label: Option<&str>) -> Volume<'_> {
        Volume {
            first_lba: 2048,
            sectors: 64 * MIB / 512,
            label,
            volume_id: 7,
        }
    }

    #[test]
    fn relit_l_etiquette_d_un_volume_vide() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(66 * MIB, 512)?;
        let vol = volume(Some("BonkVault"));
        exfat::write(&mut dev, &vol)?;
        let reader = Reader::open(&dev, vol.first_lba, vol.sectors)?;
        assert_eq!(reader.label()?.as_deref(), Some("BonkVault"));
        assert!(reader.files()?.is_empty());
        Ok(())
    }

    #[test]
    fn relit_l_arborescence_ecrite_par_bonk() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(66 * MIB, 512)?;
        let vol = volume(None);
        let files = [
            file("debian.iso", 3 * MIB),
            file("linux/arch/archlinux-x86_64.iso", 5000),
            file("linux/notes.txt", 0),
        ];
        let layout = exfat_tree::layout(&vol, 512, &files, &[])?;
        layout.write_structures(&mut dev)?;
        layout.finish(&mut dev, &vol)?;
        let found = Reader::open(&dev, vol.first_lba, vol.sectors)?.files()?;
        let paths: Vec<(&str, u64)> = found
            .iter()
            .map(|f| (f.path.as_str(), f.size_bytes))
            .collect();
        assert_eq!(
            paths,
            [
                ("debian.iso", 3 * MIB),
                ("linux/arch/archlinux-x86_64.iso", 5000),
                ("linux/notes.txt", 0)
            ]
        );
        Ok(())
    }

    #[test]
    fn refuse_un_volume_qui_n_est_pas_exfat() -> Result<(), Box<dyn std::error::Error>> {
        let dev = MemoryDevice::new(4 * MIB, 512)?;
        assert!(matches!(
            Reader::open(&dev, 0, 8192),
            Err(FormatError::Invalid(_))
        ));
        assert!(Reader::open(&dev, 8000, 8192).is_err());
        Ok(())
    }

    /// Shifts come from the drive: their sum must not overflow, whatever their values.
    #[test]
    fn des_tailles_de_grappe_aberrantes_sont_refusees() -> Result<(), Box<dyn std::error::Error>> {
        for (sector_shift, cluster_shift) in [(9u8, 255u8), (255, 255), (12, 14)] {
            let mut bytes = vec![0u8; 4096];
            bytes[3..11].copy_from_slice(b"EXFAT   ");
            bytes[108] = sector_shift;
            bytes[109] = cluster_shift;
            bytes[510] = 0x55;
            bytes[511] = 0xAA;
            let dev = MemoryDevice::from_bytes(bytes, 512)?;
            assert!(matches!(
                Reader::open(&dev, 0, 8),
                Err(FormatError::Invalid(_))
            ));
        }
        Ok(())
    }

    /// A chain that loops back on itself must end in an error, not in an endless read.
    #[test]
    fn une_chaine_qui_boucle_est_refusee() -> Result<(), Box<dyn std::error::Error>> {
        let mut dev = MemoryDevice::new(66 * MIB, 512)?;
        let vol = volume(None);
        exfat::write(&mut dev, &vol)?;
        let reader = Reader::open(&dev, vol.first_lba, vol.sectors)?;
        let entry = reader.fat + u64::from(reader.root) * 4;
        let root = reader.root;
        let base = entry - entry % 512;
        let mut sector = read_range(&dev, base, 512)?;
        let at = usize::try_from(entry - base)?;
        sector[at..at + 4].copy_from_slice(&root.to_le_bytes());
        dev.write_at(base, &sector)?;
        let reader = Reader::open(&dev, vol.first_lba, vol.sectors)?;
        assert!(matches!(reader.files(), Err(FormatError::Invalid(_))));
        Ok(())
    }
}
