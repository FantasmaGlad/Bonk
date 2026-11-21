use std::io::Read;

use crate::block::BlockDevice;
use crate::format::FormatError;
use crate::format::exfat::read::Reader;
use crate::format::exfat::tree::{entries_for, name_units};
use crate::format::exfat::{END_OF_CHAIN, ENTRY, FIRST_CLUSTER, upcase_map};
use crate::format::tree::Placed;
use crate::format::tree::{verify_contiguous, write_contiguous};

#[cfg(test)]
mod tests;
mod volume;

use volume::*;

const ENTRY_BITMAP: u8 = 0x81;
const IN_USE: u8 = 0x80;
const VOLUME_FLAGS: u64 = 106;
const VOLUME_DIRTY: u8 = 0x02;
const ATTR_HIDDEN: u8 = 0x02;

/// Contiguous, without a FAT chain, so a live system can map it at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Added {
    pub name: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub volume_offset: u64,
}

/// The partition holding the volume, in device sectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub first_lba: u64,
    pub sectors: u64,
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
    refuse_taken(&volume, &units, &upcase, name)?;
    let placed = reserve(&volume, name, size)?;
    let (first_cluster, clusters) = (placed.first_cluster, placed.clusters);
    let slot = volume.free_slots(entries_for(&units)).ok_or_else(|| {
        FormatError::TooLarge(format!("{name}: the volume's root directory is full"))
    })?;
    let set = file_entry_set(&units, &upcase, placed, size, hidden);
    set_dirty(dev, volume.start, true)?;
    let offset = if clusters == 0 {
        0
    } else {
        volume.cluster_offset(first_cluster)
    };
    let sha256 = write_contiguous(dev, offset, size, reader, advance)?;
    patch(
        dev,
        &bytes(volume.bitmap_writes(first_cluster.max(FIRST_CLUSTER), clusters, true)),
    )?;
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
        volume_offset: if clusters == 0 {
            0
        } else {
            offset - volume.start
        },
    })
}

/// exFAT compares names upper-cased: two that differ only by case are the same file.
fn refuse_taken(
    volume: &Volume,
    units: &[u16],
    upcase: &[u16],
    name: &str,
) -> Result<(), FormatError> {
    let wanted = upcased(units, upcase);
    let taken = Reader::entries(&volume.root_bytes, &mut None)
        .iter()
        .any(|raw| upcased(&raw.name.encode_utf16().collect::<Vec<_>>(), upcase) == wanted);
    if taken {
        return Err(FormatError::Name(format!("{name}: already on the volume")));
    }
    Ok(())
}

fn reserve(volume: &Volume, name: &str, size: u64) -> Result<Placed, FormatError> {
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
    Ok(Placed {
        first_cluster,
        clusters,
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
    let wanted = upcased(&name.encode_utf16().collect::<Vec<_>>(), &upcase_map());
    let slots = volume.root_bytes.as_chunks::<ENTRY>().0.to_vec();
    let found = find_root_file(&slots, &wanted)
        .ok_or_else(|| FormatError::Name(format!("{name}: not on the volume")))?;
    let freed: Vec<u32> = if found.first == 0 {
        Vec::new()
    } else if found.contiguous {
        let clusters = found.length.div_ceil(volume.cluster_bytes);
        if u64::from(found.first - FIRST_CLUSTER) + clusters > u64::from(volume.clusters) {
            return Err(invalid("exFAT file beyond the volume"));
        }
        (found.first..found.first + clusters as u32).collect()
    } else {
        Reader::open(dev, span.first_lba, span.sectors)?.chain(found.first)?
    };
    set_dirty(dev, volume.start, true)?;
    let entries: Vec<(u64, Vec<u8>)> = (found.slot..found.slot + found.count)
        .map(|at| {
            let mut entry = slots[at].to_vec();
            entry[0] &= !IN_USE;
            (volume.root_slots[at], entry)
        })
        .collect();
    patch(dev, &entries)?;
    let mut bitmap = Vec::new();
    for cluster in freed.into_iter().filter(|c| *c != END_OF_CHAIN) {
        bitmap.extend(volume.bitmap_writes(cluster, 1, false));
    }
    patch(dev, &bytes(bitmap))?;
    set_dirty(dev, volume.start, false)?;
    Ok(found.length)
}

struct Found {
    slot: usize,
    count: usize,
    first: u32,
    length: u64,
    contiguous: bool,
}

/// `wanted` is the upper-cased name.
fn find_root_file(slots: &[[u8; ENTRY]], wanted: &[u16]) -> Option<Found> {
    let upcase = upcase_map();
    let mut index = 0;
    while let Some(entry) = slots.get(index) {
        if entry[0] == 0 {
            return None;
        }
        let secondary = usize::from(entry[1]);
        if entry[0] != crate::format::exfat::ENTRY_FILE || index + 1 + secondary > slots.len() {
            index += 1;
            continue;
        }
        let set = slots[index..=index + secondary].concat();
        let raws = Reader::entries(&set, &mut None);
        if let Some(raw) = raws.first()
            && !raw.directory
            && upcased(&raw.name.encode_utf16().collect::<Vec<_>>(), &upcase) == wanted
        {
            return Some(Found {
                slot: index,
                count: secondary + 1,
                first: raw.first_cluster,
                length: raw.length,
                contiguous: raw.contiguous,
            });
        }
        index += 1 + secondary;
    }
    None
}
