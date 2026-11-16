use super::*;

/// Where the volume is on the drive and what the edit touches, read before the first write.
pub(super) struct Volume {
    pub(super) start: u64,
    pub(super) cluster_bytes: u64,
    pub(super) clusters: u32,
    pub(super) heap: u64,
    /// Absolute offset of each root entry slot, in order.
    pub(super) root_slots: Vec<u64>,
    pub(super) root_bytes: Vec<u8>,
    /// Absolute offset of each bitmap byte.
    pub(super) bitmap_offsets: Vec<u64>,
    pub(super) bitmap: Vec<u8>,
}

pub(super) fn invalid(what: impl Into<String>) -> FormatError {
    FormatError::Invalid(what.into())
}

impl Volume {
    pub(super) fn read(
        dev: &dyn BlockDevice,
        first_lba: u64,
        sectors: u64,
    ) -> Result<Self, FormatError> {
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

    pub(super) fn used(&self, cluster: u32) -> bool {
        let index = (cluster - FIRST_CLUSTER) as usize;
        self.bitmap[index / 8] & (1 << (index % 8)) != 0
    }

    /// First run of `count` free clusters: the file must be contiguous.
    pub(super) fn free_run(&self, count: u64) -> Option<u32> {
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
    pub(super) fn free_slots(&self, count: usize) -> Option<usize> {
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
    pub(super) fn bitmap_writes(&mut self, first: u32, count: u32, used: bool) -> Vec<(u64, u8)> {
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

    pub(super) fn cluster_offset(&self, cluster: u32) -> u64 {
        self.heap + u64::from(cluster - FIRST_CLUSTER) * self.cluster_bytes
    }
}

/// Writes bytes one at a time through whole device blocks: bitmap bytes and entries are scattered.
pub(super) fn patch(
    dev: &mut dyn BlockDevice,
    writes: &[(u64, Vec<u8>)],
) -> Result<(), FormatError> {
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
pub(super) fn set_dirty(
    dev: &mut dyn BlockDevice,
    start: u64,
    dirty: bool,
) -> Result<(), FormatError> {
    let flags = read_range(dev, start + VOLUME_FLAGS, 1)?[0];
    let flags = if dirty {
        flags | VOLUME_DIRTY
    } else {
        flags & !VOLUME_DIRTY
    };
    patch(dev, &[(start + VOLUME_FLAGS, vec![flags])])
}

pub(super) fn upcased(units: &[u16], upcase: &[u16]) -> Vec<u16> {
    units
        .iter()
        .map(|unit| upcase[usize::from(*unit)])
        .collect()
}

pub(super) fn bytes(writes: Vec<(u64, u8)>) -> Vec<(u64, Vec<u8>)> {
    writes
        .into_iter()
        .map(|(at, byte)| (at, vec![byte]))
        .collect()
}

pub(super) fn file_entry_set(
    units: &[u16],
    upcase: &[u16],
    placed: Placed,
    size: u64,
    hidden: bool,
) -> Vec<u8> {
    let mut set = entry_set(units, upcase, false, placed, size);
    if hidden {
        set[4] |= ATTR_HIDDEN;
        let sum = checksum16(&set, &[2, 3]);
        set[2..4].copy_from_slice(&sum.to_le_bytes());
    }
    set
}
