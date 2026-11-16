use std::fmt;
use std::io;

mod raw;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

use raw::*;

/// Positional reads: several files may read the image at once, without a shared cursor.
pub trait ReadAt {
    /// # Errors
    /// Read failure, or past the end.
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()>;
}

impl ReadAt for [u8] {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let start = usize::try_from(offset).map_err(|_| io::ErrorKind::UnexpectedEof)?;
        let end = start
            .checked_add(buf.len())
            .filter(|end| *end <= self.len())
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        buf.copy_from_slice(&self[start..end]);
        Ok(())
    }
}

#[derive(Debug)]
pub enum UdfError {
    NotUdf,
    Malformed(&'static str),
    Io(io::Error),
}

impl fmt::Display for UdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotUdf => f.write_str("no UDF filesystem"),
            Self::Malformed(what) => write!(f, "UDF unreadable: {what}"),
            Self::Io(err) => write!(f, "reading the image: {err}"),
        }
    }
}

impl std::error::Error for UdfError {}

impl From<io::Error> for UdfError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Image offset and length, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub offset: u64,
    pub len: u64,
}

/// In image extents, or inline in the file entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Extents(Vec<Extent>),
    Inline(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: Vec<String>,
    pub size: u64,
    pub content: Content,
}

/// Empty directories included.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub files: Vec<File>,
    pub directories: Vec<Vec<String>>,
}

impl Tree {
    #[must_use]
    pub fn find(&self, path: &[&str]) -> Option<&File> {
        self.files.iter().find(|file| {
            file.path.len() == path.len()
                && file
                    .path
                    .iter()
                    .zip(path)
                    .all(|(a, b)| a.eq_ignore_ascii_case(b))
        })
    }
}

struct Entry {
    directory: bool,
    size: u64,
    content: Content,
}

/// Single type-1 partition UDF, the case of Windows ISOs.
pub struct Udf<'a, R: ReadAt + ?Sized> {
    reader: &'a R,
    partition_start: u64,
    root: LongAd,
    block: u64,
}

impl<'a, R: ReadAt + ?Sized> Udf<'a, R> {
    /// # Errors
    /// [`UdfError::NotUdf`] without an anchor; [`UdfError::Malformed`] for an inconsistent structure.
    pub fn open(reader: &'a R) -> Result<Self, UdfError> {
        let mut anchor = [0; SECTOR as usize];
        if reader
            .read_exact_at(ANCHOR_SECTOR * SECTOR, &mut anchor)
            .is_err()
            || tag_id(&anchor) != Some(TAG_ANCHOR)
        {
            return Err(UdfError::NotUdf);
        }
        let sequence_len = u64::from(u32_at(&anchor, 16)?);
        let sequence_start = u64::from(u32_at(&anchor, 20)?);
        let mut partition = None;
        let mut volume = None;
        for index in 0..(sequence_len / SECTOR).min(MAX_DESCRIPTORS) {
            let mut sector = [0; SECTOR as usize];
            reader.read_exact_at((sequence_start + index) * SECTOR, &mut sector)?;
            match tag_id(&sector) {
                Some(TAG_PARTITION) => partition = Some(u64::from(u32_at(&sector, 188)?)),
                Some(TAG_LOGICAL_VOLUME) => {
                    let block = u32_at(&sector, 212)?;
                    let maps = u32_at(&sector, 268)?;
                    let map_kind = *sector
                        .get(440)
                        .ok_or(UdfError::Malformed("partition table"))?;
                    if block != SECTOR as u32 || maps != 1 || map_kind != 1 {
                        return Err(UdfError::Malformed("volume shape not supported"));
                    }
                    volume = Some(long_ad(&sector, 248)?);
                }
                Some(TAG_TERMINATOR) => break,
                _ => {}
            }
        }
        let partition_start = partition.ok_or(UdfError::Malformed("partition missing"))?;
        let file_set = volume.ok_or(UdfError::Malformed("logical volume missing"))?;
        let mut udf = Self {
            reader,
            partition_start,
            root: file_set,
            block: SECTOR,
        };
        let mut sector = [0; SECTOR as usize];
        reader.read_exact_at(udf.block_offset(file_set.block), &mut sector)?;
        if tag_id(&sector) != Some(TAG_FILE_SET) {
            return Err(UdfError::Malformed("file set descriptor"));
        }
        udf.root = long_ad(&sector, 400)?;
        Ok(udf)
    }

    fn block_offset(&self, block: u32) -> u64 {
        (self.partition_start + u64::from(block)) * self.block
    }

    fn read_entry(&self, at: LongAd) -> Result<Entry, UdfError> {
        let mut sector = [0; SECTOR as usize];
        self.reader
            .read_exact_at(self.block_offset(at.block), &mut sector)?;
        let (ea_at, ad_at, base) = match tag_id(&sector) {
            Some(TAG_FILE_ENTRY) => (168, 172, 176),
            Some(TAG_EXTENDED_FILE_ENTRY) => (208, 212, 216),
            _ => return Err(UdfError::Malformed("file entry expected")),
        };
        let directory = sector[27] == ICB_DIRECTORY;
        let flags = u16_at(&sector, 34)?;
        let size = u64_at(&sector, 56)?;
        let ea_len = u32_at(&sector, ea_at)? as usize;
        let ad_len = u32_at(&sector, ad_at)? as usize;
        let start = base + ea_len;
        let descriptors = start
            .checked_add(ad_len)
            .and_then(|end| sector.get(start..end))
            .ok_or(UdfError::Malformed("extents outside the entry"))?;
        let content = match flags & 7 {
            0 => Content::Extents(self.extents(descriptors, 8, |d| u32_at(d, 4))?),
            1 => Content::Extents(self.extents(descriptors, 16, |d| u32_at(d, 4))?),
            3 => Content::Inline(descriptors.to_vec()),
            _ => return Err(UdfError::Malformed("extent shape not supported")),
        };
        let held = match &content {
            Content::Extents(extents) => extents.iter().map(|extent| extent.len).sum(),
            Content::Inline(bytes) => bytes.len() as u64,
        };
        if held < size {
            return Err(UdfError::Malformed("file longer than its extents"));
        }
        Ok(Entry {
            directory,
            size,
            content,
        })
    }

    fn extents(
        &self,
        descriptors: &[u8],
        width: usize,
        block: impl Fn(&[u8]) -> Result<u32, UdfError>,
    ) -> Result<Vec<Extent>, UdfError> {
        let mut extents = Vec::new();
        for descriptor in descriptors.chunks_exact(width) {
            let raw = u32_at(descriptor, 0)?;
            if raw == 0 {
                break;
            }
            extents.push(Extent {
                offset: self.block_offset(block(descriptor)?),
                len: recorded_len(raw)?,
            });
        }
        Ok(extents)
    }

    fn directory_bytes(&self, entry: &Entry) -> Result<Vec<u8>, UdfError> {
        if entry.size > MAX_DIRECTORY_BYTES {
            return Err(UdfError::Malformed("directory too large"));
        }
        let mut bytes = vec![0; entry.size as usize];
        read_content(self.reader, &entry.content, 0, &mut bytes)?;
        Ok(bytes)
    }

    /// # Errors
    /// Inconsistent structure, unreadable image, or tree beyond the limits.
    pub fn tree(&self) -> Result<Tree, UdfError> {
        let mut tree = Tree::default();
        let mut pending = vec![(self.root, Vec::new())];
        while let Some((at, path)) = pending.pop() {
            let directory = self.read_entry(at)?;
            if !directory.directory {
                return Err(UdfError::Malformed("directory expected"));
            }
            let bytes = self.directory_bytes(&directory)?;
            let mut position = 0;
            while position + 38 <= bytes.len() {
                let record = &bytes[position..];
                if tag_id(record) != Some(TAG_FILE_ID) {
                    return Err(UdfError::Malformed("directory entry"));
                }
                let characteristics = record[18];
                let name_len = usize::from(record[19]);
                let use_len = usize::from(u16_at(record, 36)?);
                let total = (38 + use_len + name_len + 3) & !3;
                if total > record.len() {
                    return Err(UdfError::Malformed("truncated directory entry"));
                }
                position += total;
                if characteristics & (FID_PARENT | FID_DELETED) != 0 {
                    continue;
                }
                if tree.files.len() + tree.directories.len() >= MAX_ENTRIES {
                    return Err(UdfError::Malformed("too many files"));
                }
                let mut child = path.clone();
                child.push(decode_name(&record[38 + use_len..38 + use_len + name_len])?);
                let icb = long_ad(record, 20)?;
                if characteristics & FID_DIRECTORY != 0 {
                    if child.len() > MAX_DEPTH {
                        return Err(UdfError::Malformed("directories too deep"));
                    }
                    tree.directories.push(child.clone());
                    pending.push((icb, child));
                } else {
                    let entry = self.read_entry(icb)?;
                    tree.files.push(File {
                        path: child,
                        size: entry.size,
                        content: entry.content,
                    });
                }
            }
        }
        tree.files.sort_by(|a, b| a.path.cmp(&b.path));
        tree.directories.sort();
        Ok(tree)
    }
}

/// Reads `buf.len()` bytes of a file from `offset`, across its extents.
///
/// # Errors
/// Read failure, or past the end of the extents.
pub fn read_content<R: ReadAt + ?Sized>(
    reader: &R,
    content: &Content,
    offset: u64,
    buf: &mut [u8],
) -> io::Result<()> {
    match content {
        Content::Inline(bytes) => bytes.as_slice().read_exact_at(offset, buf),
        Content::Extents(extents) => {
            let mut skip = offset;
            let mut done = 0;
            for extent in extents {
                if skip >= extent.len {
                    skip -= extent.len;
                    continue;
                }
                let available = extent.len - skip;
                let take = available.min((buf.len() - done) as u64) as usize;
                reader.read_exact_at(extent.offset + skip, &mut buf[done..done + take])?;
                done += take;
                skip = 0;
                if done == buf.len() {
                    return Ok(());
                }
            }
            if done == buf.len() {
                Ok(())
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            }
        }
    }
}
