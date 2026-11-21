use std::collections::HashSet;

use crate::bytes::{u16_le, u32_le};
use crate::read_at::ReadAt;

const ENTRY: usize = 32;
const ATTR_VOLUME_ID: u8 = 0x08;
const ATTR_DIRECTORY: u8 = 0x10;
const ATTR_LONG_NAME: u8 = 0x0F;
const MAX_DIRECTORY_BYTES: u64 = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Fat12,
    Fat16,
    Fat32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Root {
    Fixed { offset: u64, len: u64 },
    Chain(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    directory: bool,
    cluster: u32,
    size: u32,
}

struct Bpb {
    bytes_per_sector: u64,
    per_cluster: u64,
    reserved: u64,
    fats: u64,
    root_entries: u64,
    fat_sectors: u64,
    total: u64,
    root_cluster: u32,
}

impl Bpb {
    fn parse(boot: &[u8; 512]) -> Option<Self> {
        let small_or = |small: u16, large: u32| match small {
            0 => u64::from(large),
            small => u64::from(small),
        };
        let bpb = Self {
            bytes_per_sector: u64::from(u16_le(boot, 11)),
            per_cluster: u64::from(boot[13]),
            reserved: u64::from(u16_le(boot, 14)),
            fats: u64::from(boot[16]),
            root_entries: u64::from(u16_le(boot, 17)),
            fat_sectors: small_or(u16_le(boot, 22), u32_le(boot, 36)),
            total: small_or(u16_le(boot, 19), u32_le(boot, 32)),
            root_cluster: u32_le(boot, 44),
        };
        let valid = boot[510..] == [0x55, 0xAA]
            && matches!(bpb.bytes_per_sector, 512 | 1024 | 2048 | 4096)
            && bpb.per_cluster.is_power_of_two()
            && bpb.per_cluster <= 128
            && bpb.reserved > 0
            && bpb.fats > 0;
        valid.then_some(bpb)
    }
}

// Read-only FAT12, FAT16 and FAT32, Microsoft FAT specification, sections 3 and 4; offsets are relative to `base`.
pub struct Fat<'a, R: ReadAt + ?Sized> {
    source: &'a R,
    base: u64,
    kind: Kind,
    cluster_bytes: u64,
    fat_offset: u64,
    data_offset: u64,
    clusters: u32,
    root: Root,
}

impl<'a, R: ReadAt + ?Sized> Fat<'a, R> {
    pub fn open(source: &'a R, base: u64) -> Option<Self> {
        let mut boot = [0u8; 512];
        source.read_exact_at(base, &mut boot).ok()?;
        let bpb = Bpb::parse(&boot)?;
        let root_sectors = (bpb.root_entries * ENTRY as u64).div_ceil(bpb.bytes_per_sector);
        let root_start = bpb.reserved + bpb.fats * bpb.fat_sectors;
        let data_start = root_start + root_sectors;
        let clusters = u32::try_from(bpb.total.checked_sub(data_start)? / bpb.per_cluster).ok()?;
        let kind = match clusters {
            0..4085 => Kind::Fat12,
            4085..65525 => Kind::Fat16,
            _ => Kind::Fat32,
        };
        let root = match (kind, bpb.root_entries) {
            (Kind::Fat32, 0) => Root::Chain(bpb.root_cluster),
            (Kind::Fat12 | Kind::Fat16, 1..) => Root::Fixed {
                offset: root_start * bpb.bytes_per_sector,
                len: root_sectors * bpb.bytes_per_sector,
            },
            _ => return None,
        };
        let fat = Self {
            source,
            base,
            kind,
            cluster_bytes: bpb.per_cluster * bpb.bytes_per_sector,
            fat_offset: bpb.reserved * bpb.bytes_per_sector,
            data_offset: data_start * bpb.bytes_per_sector,
            clusters,
            root,
        };
        match root {
            Root::Chain(first) if !fat.is_data_cluster(first) => None,
            _ => Some(fat),
        }
    }

    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
    }

    // Short 8.3 names only, compared without case: what removable-media loader paths use.
    #[must_use]
    pub fn read(&self, path: &str, limit: u64) -> Option<Vec<u8>> {
        let mut directory = self.root_bytes()?;
        let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();
        while let Some(part) = parts.next() {
            let entry = find(&directory, &short_name(part)?, self.kind)?;
            if parts.peek().is_none() {
                return self.file(entry, limit);
            }
            if !entry.directory {
                return None;
            }
            directory = self.chain(entry.cluster, MAX_DIRECTORY_BYTES, false)?;
        }
        None
    }

    fn file(&self, entry: Entry, limit: u64) -> Option<Vec<u8>> {
        let size = u64::from(entry.size);
        if entry.directory || size > limit {
            return None;
        }
        if size == 0 {
            return Some(Vec::new());
        }
        self.chain(entry.cluster, size, true)
    }

    fn root_bytes(&self) -> Option<Vec<u8>> {
        match self.root {
            Root::Fixed { offset, len } => self.bytes(offset, len),
            Root::Chain(first) => self.chain(first, MAX_DIRECTORY_BYTES, false),
        }
    }

    fn chain(&self, first: u32, wanted: u64, exact: bool) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut cluster = first;
        loop {
            if !self.is_data_cluster(cluster) || !seen.insert(cluster) {
                return None;
            }
            let offset = self.data_offset + u64::from(cluster - 2) * self.cluster_bytes;
            let take = self.cluster_bytes.min(wanted - out.len() as u64);
            out.extend(self.bytes(offset, take)?);
            if out.len() as u64 == wanted {
                return Some(out);
            }
            match self.next(cluster)? {
                Some(next) => cluster = next,
                None => return (!exact).then_some(out),
            }
        }
    }

    fn next(&self, cluster: u32) -> Option<Option<u32>> {
        let index = u64::from(cluster);
        let (value, end) = match self.kind {
            Kind::Fat12 => {
                let pair = self.bytes(self.fat_offset + index + index / 2, 2)?;
                let packed = u16::from_le_bytes([pair[0], pair[1]]);
                let value = if cluster.is_multiple_of(2) {
                    packed & 0x0FFF
                } else {
                    packed >> 4
                };
                (u32::from(value), 0x0FF8)
            }
            Kind::Fat16 => {
                let pair = self.bytes(self.fat_offset + index * 2, 2)?;
                (u32::from(u16::from_le_bytes([pair[0], pair[1]])), 0xFFF8)
            }
            Kind::Fat32 => {
                let quad = self.bytes(self.fat_offset + index * 4, 4)?;
                (u32_le(&quad, 0) & 0x0FFF_FFFF, 0x0FFF_FFF8)
            }
        };
        if value >= end {
            Some(None)
        } else {
            self.is_data_cluster(value).then_some(Some(value))
        }
    }

    fn is_data_cluster(&self, cluster: u32) -> bool {
        (2..self.clusters.saturating_add(2)).contains(&cluster)
    }

    fn bytes(&self, offset: u64, len: u64) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; usize::try_from(len).ok()?];
        self.source
            .read_exact_at(self.base.checked_add(offset)?, &mut buf)
            .ok()?;
        Some(buf)
    }
}

fn short_name(part: &str) -> Option<[u8; 11]> {
    let (base, extension) = part.rsplit_once('.').unwrap_or((part, ""));
    let plain = |text: &str, max: usize| {
        !text.is_empty()
            && text.len() <= max
            && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    };
    if !plain(base, 8) || !(extension.is_empty() || plain(extension, 3)) {
        return None;
    }
    let mut name = [b' '; 11];
    name[..base.len()].copy_from_slice(base.to_ascii_uppercase().as_bytes());
    name[8..8 + extension.len()].copy_from_slice(extension.to_ascii_uppercase().as_bytes());
    Some(name)
}

fn find(directory: &[u8], name: &[u8; 11], kind: Kind) -> Option<Entry> {
    for raw in directory.as_chunks::<ENTRY>().0 {
        match raw[0] {
            0x00 => return None,
            0xE5 => continue,
            _ => {}
        }
        let attributes = raw[11];
        if attributes & ATTR_LONG_NAME == ATTR_LONG_NAME
            || attributes & ATTR_VOLUME_ID != 0
            || !raw[..11].eq_ignore_ascii_case(name)
        {
            continue;
        }
        let high = if kind == Kind::Fat32 {
            u32::from(u16_le(raw, 20)) << 16
        } else {
            0
        };
        return Some(Entry {
            directory: attributes & ATTR_DIRECTORY != 0,
            cluster: high | u32::from(u16_le(raw, 26)),
            size: u32_le(raw, 28),
        });
    }
    None
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
