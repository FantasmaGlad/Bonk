use std::io::Read;

use super::FormatError;
use super::exfat::{END_OF_CHAIN, ENTRY, FIRST_CLUSTER, checksum16, upcase_map};
use super::exfat_read::Reader;
use super::exfat_tree::{Placed, entries_for, entry_set, name_units};
use super::tree::{verify_contiguous, write_contiguous};
use crate::block::BlockDevice;
use crate::partition::read_range;

const ENTRY_BITMAP: u8 = 0x81;
const IN_USE: u8 = 0x80;
const VOLUME_FLAGS: u64 = 106;
const VOLUME_DIRTY: u8 = 0x02;
const ATTR_HIDDEN: u8 = 0x02;

/// A file added to the root of an existing exFAT volume, written by any system: contiguous, without a FAT
/// chain, so a live system can map it at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
    /// Where the data starts, in bytes from the start of the volume.
    pub volume_offset: u64,
}

/// The partition holding the volume, in device sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub first_lba: u64,
    pub sectors: u64,
}

/// Where the volume is on the drive and what the edit touches, read before the first write.
struct Volume {
    start: u64,
    cluster_bytes: u64,
    clusters: u32,
    heap: u64,
    /// Absolute offset of each root entry slot, in order.
    root_slots: Vec<u64>,
    root_bytes: Vec<u8>,
    /// Absolute offset of each bitmap byte.
    bitmap_offsets: Vec<u64>,
    bitmap: Vec<u8>,
}

fn invalid(what: impl Into<String>) -> FormatError {
    FormatError::Invalid(what.into())
}

impl Volume {
    fn read(dev: &dyn BlockDevice, first_lba: u64, sectors: u64) -> Result<Self, FormatError> {
        let reader = Reader::open(dev, first_lba, sectors)?;
        let start = first_lba * u64::from(dev.block_size());
        let cluster_bytes = reader.cluster_bytes();
        let root_chain = reader.chain(reader.root())?;
        let slots_per_cluster = cluster_bytes / ENTRY as u64;
        let root_slots: Vec<u64> = root_chain
            .iter()
            .flat_map(|cluster| {
                let base = reader.offset(*cluster);
                (0..slots_per_cluster).map(move |slot| base + slot * ENTRY as u64)
            })
            .collect();
        let root_bytes = reader.read(reader.root(), None, false)?;
        let bitmap_entry = root_bytes
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .find(|entry| entry[0] == ENTRY_BITMAP)
            .ok_or_else(|| invalid("exFAT volume without an allocation bitmap"))?;
        let bitmap_first = u32::from_le_bytes([
            bitmap_entry[20],
            bitmap_entry[21],
            bitmap_entry[22],
            bitmap_entry[23],
        ]);
        let clusters = reader.clusters();
        let bitmap_len = u64::from(clusters).div_ceil(8);
        let bitmap_chain = reader.chain(bitmap_first)?;
        let bitmap_offsets: Vec<u64> = bitmap_chain
            .iter()
            .flat_map(|cluster| {
                let base = reader.offset(*cluster);
                (0..cluster_bytes).map(move |byte| base + byte)
            })
            .take(usize::try_from(bitmap_len).map_err(|_| invalid("exFAT bitmap too large"))?)
            .collect();
        if (bitmap_offsets.len() as u64) < bitmap_len {
            return Err(invalid("exFAT allocation bitmap shorter than the volume"));
        }
        let bitmap = reader.read(bitmap_first, Some(bitmap_len), false)?;
        Ok(Self {
            start,
            cluster_bytes,
            clusters,
            heap: reader.offset(FIRST_CLUSTER),
            root_slots,
            root_bytes,
            bitmap_offsets,
            bitmap,
        })
    }

    fn used(&self, cluster: u32) -> bool {
        let index = (cluster - FIRST_CLUSTER) as usize;
        self.bitmap[index / 8] & (1 << (index % 8)) != 0
    }

    /// First run of `count` free clusters: the file must be contiguous.
    fn free_run(&self, count: u64) -> Option<u32> {
        let mut run = 0u64;
        for cluster in FIRST_CLUSTER..FIRST_CLUSTER + self.clusters {
            if self.used(cluster) {
                run = 0;
                continue;
            }
            run += 1;
            if run == count {
                return Some(cluster + 1 - count as u32);
            }
        }
        None
    }

    /// First `count` consecutive root slots not in use.
    fn free_slots(&self, count: usize) -> Option<usize> {
        let types: Vec<u8> = self
            .root_bytes
            .as_chunks::<ENTRY>()
            .0
            .iter()
            .map(|entry| entry[0])
            .collect();
        let mut run = 0;
        for (index, kind) in types.iter().enumerate().take(self.root_slots.len()) {
            if kind & IN_USE != 0 {
                run = 0;
                continue;
            }
            run += 1;
            if run == count {
                return Some(index + 1 - count);
            }
        }
        None
    }

    /// Bitmap bytes changed by marking `clusters` from `first`, with their offsets.
    fn bitmap_writes(&mut self, first: u32, count: u32, used: bool) -> Vec<(u64, u8)> {
        let mut touched = std::collections::BTreeSet::new();
        for cluster in first..first + count {
            let index = (cluster - FIRST_CLUSTER) as usize;
            if used {
                self.bitmap[index / 8] |= 1 << (index % 8);
            } else {
                self.bitmap[index / 8] &= !(1 << (index % 8));
            }
            touched.insert(index / 8);
        }
        touched
            .into_iter()
            .map(|byte| (self.bitmap_offsets[byte], self.bitmap[byte]))
            .collect()
    }

    fn cluster_offset(&self, cluster: u32) -> u64 {
        self.heap + u64::from(cluster - FIRST_CLUSTER) * self.cluster_bytes
    }
}

/// Writes bytes one at a time through whole device blocks: bitmap bytes and entries are scattered.
fn patch(dev: &mut dyn BlockDevice, writes: &[(u64, Vec<u8>)]) -> Result<(), FormatError> {
    let block = u64::from(dev.block_size());
    for (offset, bytes) in writes {
        let start = offset - offset % block;
        let end = (offset + bytes.len() as u64).div_ceil(block) * block;
        let mut buffer = read_range(dev, start, end - start)?;
        let at = (offset - start) as usize;
        buffer[at..at + bytes.len()].copy_from_slice(bytes);
        dev.write_at(start, &buffer)?;
    }
    Ok(())
}

/// The dirty flag tells a system to check the volume if Bonk stops halfway; it is outside the boot checksum.
fn set_dirty(dev: &mut dyn BlockDevice, start: u64, dirty: bool) -> Result<(), FormatError> {
    let flags = read_range(dev, start + VOLUME_FLAGS, 1)?[0];
    let flags = if dirty {
        flags | VOLUME_DIRTY
    } else {
        flags & !VOLUME_DIRTY
    };
    patch(dev, &[(start + VOLUME_FLAGS, vec![flags])])
}

fn upcased(units: &[u16], upcase: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|unit| upcase[usize::from(*unit)])
        .collect()
}

/// Adds `name` (root, `size` bytes read from `reader`); the caller flushes and reads it back with
/// [`verify_file`]. Data, then bitmap, then entries: interrupted, the volume holds at worst clusters marked used
/// by no file.
///
/// # Errors
/// [`FormatError::Name`] for a refused or existing name, [`FormatError::TooLarge`] without enough contiguous
/// space or root slots, read or write failure, or stop requested.
pub fn add_file(
    dev: &mut dyn BlockDevice,
    span: Span,
    name: &str,
    size: u64,
    hidden: bool,
    reader: &mut dyn Read,
    advance: &mut dyn FnMut(u64) -> bool,
) -> Result<Added, FormatError> {
    let mut volume = Volume::read(dev, span.first_lba, span.sectors)?;
    let units = name_units(name)?;
    let upcase = upcase_map();
    let wanted = upcased(&units, &upcase);
    for raw in Reader::entries(&volume.root_bytes, &mut None) {
        let existing: Vec<u16> = raw.name.encode_utf16().collect();
        if upcased(&existing, &upcase) == wanted {
            return Err(FormatError::Name(format!("{name}: already on the volume")));
        }
    }
    let count = size.div_ceil(volume.cluster_bytes);
    let first_cluster = if count == 0 {
        0
    } else {
        volume.free_run(count).ok_or_else(|| {
            FormatError::TooLarge(format!(
                "{name}: no free contiguous space of {size} bytes on the volume"
            ))
        })?
    };
    let clusters = u32::try_from(count).map_err(|_| FormatError::TooLarge(name.to_owned()))?;
    let slot = volume.free_slots(entries_for(&units)).ok_or_else(|| {
        FormatError::TooLarge(format!("{name}: the volume's root directory is full"))
    })?;
    let mut set = entry_set(
        &units,
        &upcase,
        false,
        Placed {
            first_cluster,
            clusters,
        },
        size,
    );
    if hidden {
        set[4] |= ATTR_HIDDEN;
        let sum = checksum16(&set, &[2, 3]);
        set[2..4].copy_from_slice(&sum.to_le_bytes());
    }

    set_dirty(dev, volume.start, true)?;
    let offset = if count == 0 {
        0
    } else {
        volume.cluster_offset(first_cluster)
    };
    let sha256 = write_contiguous(dev, offset, size, reader, advance)?;
    let bitmap: Vec<(u64, Vec<u8>)> = volume
        .bitmap_writes(first_cluster.max(FIRST_CLUSTER), clusters, true)
        .into_iter()
        .map(|(at, byte)| (at, vec![byte]))
        .collect();
    patch(dev, &bitmap)?;
    let entries: Vec<(u64, Vec<u8>)> = set
        .as_chunks::<ENTRY>()
        .0
        .iter()
        .enumerate()
        .map(|(index, entry)| (volume.root_slots[slot + index], entry.to_vec()))
        .collect();
    patch(dev, &entries)?;
    set_dirty(dev, volume.start, false)?;
    Ok(Added {
        name: name.to_owned(),
        size_bytes: size,
        sha256,
        volume_offset: if count == 0 { 0 } else { offset - volume.start },
    })
}

/// Reads `name` back from the root and compares it with the SHA-256 it was written with.
///
/// # Errors
/// [`FormatError::Name`] when absent, read failure, mismatch, or stop requested.
pub fn verify_file(
    dev: &dyn BlockDevice,
    span: Span,
    name: &str,
    sha256: &str,
    advance: &mut dyn FnMut(u64) -> bool,
) -> Result<(), FormatError> {
    let volume = Volume::read(dev, span.first_lba, span.sectors)?;
    let raw = Reader::entries(&volume.root_bytes, &mut None)
        .into_iter()
        .find(|raw| raw.name == name && !raw.directory)
        .ok_or_else(|| FormatError::Name(format!("{name}: not on the volume")))?;
    if raw.length == 0 {
        return verify_contiguous(dev, 0, 0, sha256, name, advance);
    }
    if !raw.contiguous {
        return Err(invalid(format!("{name}: not contiguous")));
    }
    verify_contiguous(
        dev,
        volume.cluster_offset(raw.first_cluster),
        raw.length,
        sha256,
        name,
        advance,
    )
}

/// Removes `name` from the root: its entries marked unused, its clusters freed in the bitmap. Returns the bytes
/// freed.
///
/// # Errors
/// [`FormatError::Name`] when the root has no such file, read or write failure.
pub fn remove_file(dev: &mut dyn BlockDevice, span: Span, name: &str) -> Result<u64, FormatError> {
    let mut volume = Volume::read(dev, span.first_lba, span.sectors)?;
    let upcase = upcase_map();
    let wanted = upcased(&name.encode_utf16().collect::<Vec<_>>(), &upcase);
    let slots = volume.root_bytes.as_chunks::<ENTRY>().0.to_vec();
    let mut index = 0;
    let found = loop {
        let Some(entry) = slots.get(index) else {
            break None;
        };
        if entry[0] == 0 {
            break None;
        }
        let secondary = usize::from(entry[1]);
        if entry[0] != super::exfat::ENTRY_FILE || index + 1 + secondary > slots.len() {
            index += 1;
            continue;
        }
        let set = slots[index..=index + secondary].concat();
        let raws = Reader::entries(&set, &mut None);
        if let Some(raw) = raws.first()
            && !raw.directory
            && upcased(&raw.name.encode_utf16().collect::<Vec<_>>(), &upcase) == wanted
        {
            break Some((
                index,
                secondary + 1,
                raw.first_cluster,
                raw.length,
                raw.contiguous,
            ));
        }
        index += 1 + secondary;
    };
    let (slot, count, first, length, contiguous) =
        found.ok_or_else(|| FormatError::Name(format!("{name}: not on the volume")))?;
    let freed: Vec<u32> = if first == 0 {
        Vec::new()
    } else if contiguous {
        let clusters = length.div_ceil(volume.cluster_bytes);
        if u64::from(first - FIRST_CLUSTER) + clusters > u64::from(volume.clusters) {
            return Err(invalid("exFAT file beyond the volume"));
        }
        (first..first + clusters as u32).collect()
    } else {
        Reader::open(dev, span.first_lba, span.sectors)?.chain(first)?
    };

    set_dirty(dev, volume.start, true)?;
    let entries: Vec<(u64, Vec<u8>)> = (slot..slot + count)
        .map(|at| {
            let mut entry = slots[at].to_vec();
            entry[0] &= !IN_USE;
            (volume.root_slots[at], entry)
        })
        .collect();
    patch(dev, &entries)?;
    let mut bitmap = Vec::new();
    for cluster in freed {
        if cluster != END_OF_CHAIN {
            bitmap.extend(volume.bitmap_writes(cluster, 1, false));
        }
    }
    let bitmap: Vec<(u64, Vec<u8>)> = bitmap
        .into_iter()
        .map(|(at, byte)| (at, vec![byte]))
        .collect();
    patch(dev, &bitmap)?;
    set_dirty(dev, volume.start, false)?;
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;
    use crate::format::exfat::{self, Volume as NewVolume};

    const MIB: u64 = 1 << 20;
    const VOLUME: NewVolume<'static> = NewVolume {
        first_lba: 2048,
        sectors: 64 * MIB / 512,
        label: Some("BonkVault"),
        volume_id: 7,
    };
    const SPAN: Span = Span {
        first_lba: VOLUME.first_lba,
        sectors: VOLUME.sectors,
    };

    fn empty() -> Result<MemoryDevice, FormatError> {
        let mut dev = MemoryDevice::new(66 * MIB, 512).map_err(FormatError::from)?;
        exfat::write(&mut dev, &VOLUME)?;
        Ok(dev)
    }

    fn add(dev: &mut MemoryDevice, name: &str, data: &[u8]) -> Result<Added, FormatError> {
        let mut reader = data;
        add_file(
            dev,
            SPAN,
            name,
            data.len() as u64,
            false,
            &mut reader,
            &mut |_| true,
        )
    }

    fn listed(dev: &MemoryDevice) -> Result<Vec<(String, u64)>, FormatError> {
        Ok(Reader::open(dev, VOLUME.first_lba, VOLUME.sectors)?
            .files()?
            .into_iter()
            .map(|file| (file.path, file.size_bytes))
            .collect())
    }

    fn content(dev: &MemoryDevice, name: &str) -> Result<Vec<u8>, FormatError> {
        let volume = Volume::read(dev, VOLUME.first_lba, VOLUME.sectors)?;
        let raw = Reader::entries(&volume.root_bytes, &mut None)
            .into_iter()
            .find(|raw| raw.name == name)
            .ok_or_else(|| invalid("absent"))?;
        Ok(read_range(
            dev,
            volume.cluster_offset(raw.first_cluster),
            raw.length,
        )?)
    }

    #[test]
    fn un_fichier_ajoute_est_relu_contigu_et_le_volume_reste_propre() -> Result<(), FormatError> {
        let mut dev = empty()?;
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i * 7) as u8).collect();
        let added = add(&mut dev, "debian-live.iso", &data)?;
        assert_eq!(added.size_bytes, data.len() as u64);
        verify_file(&dev, SPAN, "debian-live.iso", &added.sha256, &mut |_| true)?;
        assert!(
            verify_file(&dev, SPAN, "debian-live.iso", &"0".repeat(64), &mut |_| {
                true
            })
            .is_err()
        );
        add(&mut dev, "vide.txt", &[])?;
        assert_eq!(
            listed(&dev)?,
            [
                ("debian-live.iso".to_owned(), data.len() as u64),
                ("vide.txt".to_owned(), 0)
            ]
        );
        assert_eq!(content(&dev, "debian-live.iso")?, data);
        let at = VOLUME.first_lba * 512 + added.volume_offset;
        assert_eq!(read_range(&dev, at, data.len() as u64)?, data);
        let start = VOLUME.first_lba * 512;
        assert_eq!(
            read_range(&dev, start + VOLUME_FLAGS, 1)?[0] & VOLUME_DIRTY,
            0
        );
        Ok(())
    }

    #[test]
    fn un_nom_deja_present_ou_refuse_n_ecrit_rien() -> Result<(), FormatError> {
        let mut dev = empty()?;
        add(&mut dev, "Ubuntu.iso", b"abc")?;
        let before = dev.as_bytes().to_vec();
        assert!(matches!(
            add(&mut dev, "UBUNTU.ISO", b"x"),
            Err(FormatError::Name(_))
        ));
        assert!(matches!(
            add(&mut dev, "a/b.iso", b"x"),
            Err(FormatError::Name(_))
        ));
        assert_eq!(dev.as_bytes(), before.as_slice());
        Ok(())
    }

    #[test]
    fn sans_place_contigue_rien_n_est_ecrit() -> Result<(), FormatError> {
        let mut dev = empty()?;
        let before = dev.as_bytes().to_vec();
        let mut reader = std::io::repeat(0);
        let refused = add_file(
            &mut dev,
            SPAN,
            "trop.iso",
            100 * MIB,
            false,
            &mut reader,
            &mut |_| true,
        );
        assert!(matches!(refused, Err(FormatError::TooLarge(_))));
        assert_eq!(dev.as_bytes(), before.as_slice());
        Ok(())
    }

    #[test]
    fn un_fichier_retire_libere_sa_place() -> Result<(), FormatError> {
        let mut dev = empty()?;
        let big = vec![1u8; 40 * MIB as usize];
        add(&mut dev, "a.iso", &big)?;
        assert!(matches!(
            add(&mut dev, "b.iso", &big),
            Err(FormatError::TooLarge(_))
        ));
        assert_eq!(remove_file(&mut dev, SPAN, "A.ISO")?, big.len() as u64);
        assert!(listed(&dev)?.is_empty());
        add(&mut dev, "b.iso", &big)?;
        assert_eq!(listed(&dev)?, [("b.iso".to_owned(), big.len() as u64)]);
        assert!(matches!(
            remove_file(&mut dev, SPAN, "absent.iso"),
            Err(FormatError::Name(_))
        ));
        Ok(())
    }
}
