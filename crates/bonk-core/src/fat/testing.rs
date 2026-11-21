#![allow(clippy::expect_used)]

use std::collections::BTreeMap;

use super::Kind;
use crate::MemoryDevice;
use crate::bytes::{put_u16, put_u32};
use crate::format::NewFile;
use crate::format::fat32;

const SECTOR: usize = 512;
const FATS: usize = 2;

pub struct Small {
    pub image: Vec<u8>,
    kind: Kind,
    fat_sectors: usize,
    data_start: usize,
    next: u32,
}

impl Small {
    pub fn fat_offset(&self) -> usize {
        SECTOR
    }

    pub fn set_fat(&mut self, cluster: u32, value: u32) {
        for copy in 0..FATS {
            let fat = &mut self.image[SECTOR * (1 + copy * self.fat_sectors)..];
            let index = cluster as usize;
            match self.kind {
                Kind::Fat12 => {
                    let at = index + index / 2;
                    let packed = u16::from_le_bytes([fat[at], fat[at + 1]]);
                    let packed = if index.is_multiple_of(2) {
                        (packed & 0xF000) | (value as u16 & 0x0FFF)
                    } else {
                        (packed & 0x000F) | ((value as u16) << 4)
                    };
                    put_u16(fat, at, packed);
                }
                Kind::Fat16 => put_u16(fat, index * 2, value as u16),
                Kind::Fat32 => put_u32(fat, index * 4, value),
            }
        }
    }

    fn end(&self) -> u32 {
        match self.kind {
            Kind::Fat12 => 0x0FFF,
            Kind::Fat16 => 0xFFFF,
            Kind::Fat32 => 0x0FFF_FFFF,
        }
    }

    fn alloc(&mut self, len: usize) -> u32 {
        if len == 0 {
            return 0;
        }
        let first = self.next;
        let count = len.div_ceil(SECTOR) as u32;
        for cluster in first..first + count {
            let next = if cluster + 1 == first + count {
                self.end()
            } else {
                cluster + 1
            };
            self.set_fat(cluster, next);
        }
        self.next += count;
        first
    }

    fn cluster_at(&self, cluster: u32) -> usize {
        (self.data_start + cluster as usize - 2) * SECTOR
    }
}

fn short(name: &str) -> [u8; 11] {
    let (base, ext) = name.rsplit_once('.').unwrap_or((name, ""));
    let mut out = [b' '; 11];
    out[..base.len()].copy_from_slice(base.as_bytes());
    out[8..8 + ext.len()].copy_from_slice(ext.as_bytes());
    out
}

fn entry(name: &str, directory: bool, cluster: u32, size: usize) -> [u8; 32] {
    let mut raw = [0u8; 32];
    raw[..11].copy_from_slice(&short(name));
    raw[11] = if directory { 0x10 } else { 0x20 };
    put_u16(&mut raw, 26, cluster as u16);
    put_u32(&mut raw, 28, size as u32);
    raw
}

// FAT12 or FAT16 of 512-byte clusters, names given as they are stored, one cluster per directory.
pub fn small(kind: Kind, files: &[(&str, &[u8])]) -> Small {
    let (total, entry_bits): (usize, usize) = match kind {
        Kind::Fat12 => (2880, 12),
        Kind::Fat16 => (8000, 16),
        Kind::Fat32 => unreachable!("FAT32 comes from the writer"),
    };
    let fat_sectors = ((total + 2) * entry_bits / 8).div_ceil(SECTOR);
    let data_start = 1 + FATS * fat_sectors + 1;
    let mut fat = Small {
        image: vec![0u8; total * SECTOR],
        kind,
        fat_sectors,
        data_start,
        next: 2,
    };
    boot_sector(&mut fat.image, total, fat_sectors);
    let mut directories: BTreeMap<String, u32> = BTreeMap::new();
    let mut rows: BTreeMap<String, Vec<[u8; 32]>> = BTreeMap::new();
    for (path, data) in files {
        let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
        let mut walked = String::new();
        for part in parent.split('/').filter(|part| !part.is_empty()) {
            let holder = walked.clone();
            walked = if walked.is_empty() {
                part.to_owned()
            } else {
                format!("{walked}/{part}")
            };
            if !directories.contains_key(&walked) {
                let cluster = fat.alloc(SECTOR);
                directories.insert(walked.clone(), cluster);
                rows.entry(holder)
                    .or_default()
                    .push(entry(part, true, cluster, 0));
            }
        }
        let cluster = fat.alloc(data.len());
        if cluster != 0 {
            let at = fat.cluster_at(cluster);
            fat.image[at..at + data.len()].copy_from_slice(data);
        }
        rows.entry(parent.to_owned())
            .or_default()
            .push(entry(name, false, cluster, data.len()));
    }
    for (directory, entries) in rows {
        let at = match directories.get(&directory) {
            Some(cluster) => fat.cluster_at(*cluster),
            None => (1 + FATS * fat_sectors) * SECTOR,
        };
        for (index, raw) in entries.iter().enumerate() {
            fat.image[at + index * 32..at + index * 32 + 32].copy_from_slice(raw);
        }
    }
    fat
}

fn boot_sector(image: &mut [u8], total: usize, fat_sectors: usize) {
    let boot = &mut image[..SECTOR];
    boot[0] = 0xEB;
    put_u16(boot, 11, SECTOR as u16);
    boot[13] = 1;
    put_u16(boot, 14, 1);
    boot[16] = FATS as u8;
    put_u16(boot, 17, 16);
    put_u16(boot, 19, total as u16);
    boot[21] = 0xF8;
    put_u16(boot, 22, fat_sectors as u16);
    boot[510] = 0x55;
    boot[511] = 0xAA;
}

pub fn fat32(files: &[(&str, &[u8])]) -> Vec<u8> {
    let sectors = 80_000u64;
    let mut dev = MemoryDevice::new(sectors * 512, 512).expect("device");
    let volume = fat32::Volume {
        first_lba: 0,
        sectors,
        label: Some("EFI"),
        volume_id: 1,
        bios_boot: false,
    };
    let new: Vec<NewFile> = files
        .iter()
        .map(|(path, data)| NewFile {
            path: path.split('/').map(str::to_owned).collect(),
            size: data.len() as u64,
        })
        .collect();
    let layout = fat32::tree::layout(&volume, 512, &new, &[]).expect("layout");
    layout.write_structures(&mut dev).expect("structures");
    for (index, (_, data)) in files.iter().enumerate() {
        layout
            .write_file(&mut dev, index, &mut &data[..], &mut |_| true)
            .expect("file");
    }
    layout.finish(&mut dev, &volume).expect("finish");
    dev.as_bytes().to_vec()
}
