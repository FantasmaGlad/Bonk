use crate::bytes::{get_u16, get_u32, get_u64};
use std::fmt;
use std::io;

/// Optical media sector. The main anchor is always at sector 256.
const SECTOR: u64 = 2048;
const ANCHOR_SECTOR: u64 = 256;
const MAX_DESCRIPTORS: u64 = 4096;
const MAX_DEPTH: usize = 32;
const MAX_ENTRIES: usize = 1_000_000;
const MAX_DIRECTORY_BYTES: u64 = 64 << 20;

const TAG_PARTITION: u16 = 5;
const TAG_LOGICAL_VOLUME: u16 = 6;
const TAG_TERMINATOR: u16 = 8;
const TAG_ANCHOR: u16 = 2;
const TAG_FILE_SET: u16 = 256;
const TAG_FILE_ID: u16 = 257;
const TAG_FILE_ENTRY: u16 = 261;
const TAG_EXTENDED_FILE_ENTRY: u16 = 266;

const ICB_DIRECTORY: u8 = 4;
const FID_DIRECTORY: u8 = 0x02;
const FID_DELETED: u8 = 0x04;
const FID_PARENT: u8 = 0x08;

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

const OUTSIDE: UdfError = UdfError::Malformed("field outside the descriptor");

fn u16_at(data: &[u8], at: usize) -> Result<u16, UdfError> {
    get_u16(data, at).ok_or(OUTSIDE)
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, UdfError> {
    get_u32(data, at).ok_or(OUTSIDE)
}

fn u64_at(data: &[u8], at: usize) -> Result<u64, UdfError> {
    get_u64(data, at).ok_or(OUTSIDE)
}

/// Id of a descriptor whose tag checksum is valid; the version byte is not checked.
fn tag_id(sector: &[u8]) -> Option<u16> {
    let head = sector.get(..16)?;
    let sum = head
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != 4)
        .fold(0u8, |sum, (_, byte)| sum.wrapping_add(*byte));
    (sum == head[4]).then(|| u16::from_le_bytes([head[0], head[1]]))
}

#[derive(Clone, Copy)]
struct LongAd {
    block: u32,
}

fn long_ad(data: &[u8], at: usize) -> Result<LongAd, UdfError> {
    Ok(LongAd {
        block: u32_at(data, at + 4)?,
    })
}

/// The extent type is in the two high bits of the length; 0: recorded.
fn recorded_len(raw: u32) -> Result<u64, UdfError> {
    if raw >> 30 == 0 {
        Ok(u64::from(raw))
    } else {
        Err(UdfError::Malformed("file extent not recorded"))
    }
}

/// First byte 8: Latin-1; 16: big-endian UTF-16.
fn decode_name(raw: &[u8]) -> Result<String, UdfError> {
    let name = match raw.split_first() {
        Some((8, rest)) => rest.iter().map(|b| char::from(*b)).collect(),
        Some((16, rest)) if rest.len().is_multiple_of(2) => {
            let units = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c));
            char::decode_utf16(units)
                .collect::<Result<String, _>>()
                .map_err(|_| UdfError::Malformed("invalid file name"))?
        }
        _ => return Err(UdfError::Malformed("invalid file name")),
    };
    // A name from the image becomes a path: it must not escape.
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', '\0']) {
        return Err(UdfError::Malformed("invalid file name"));
    }
    Ok(name)
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

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::*;

    fn tag(sector: &mut [u8], id: u16) {
        sector[0..2].copy_from_slice(&id.to_le_bytes());
        sector[2] = 2;
        let sum = sector[..16]
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 4)
            .fold(0u8, |s, (_, b)| s.wrapping_add(*b));
        sector[4] = sum;
    }

    fn fid(characteristics: u8, name: &str, block: u32) -> Vec<u8> {
        let mut encoded = vec![8];
        encoded.extend(name.bytes());
        let name_len = if name.is_empty() { 0 } else { encoded.len() };
        let mut record = vec![0; (38 + name_len + 3) & !3];
        record[18] = characteristics;
        record[19] = name_len as u8;
        record[20..24].copy_from_slice(&2048u32.to_le_bytes());
        record[24..28].copy_from_slice(&block.to_le_bytes());
        record[38..38 + name_len].copy_from_slice(&encoded[..name_len]);
        tag(&mut record, TAG_FILE_ID);
        record
    }

    const P: usize = 300;
    const BLOCK: usize = 2048;

    fn block(b: usize) -> usize {
        (P + b) * BLOCK
    }

    /// Anchor at sector 256 pointing to the descriptor sequence at 257: partition at block `P`, one
    /// logical volume, terminator.
    fn volume_descriptors(img: &mut [u8]) {
        let s = 256 * BLOCK;
        img[s + 16..s + 20].copy_from_slice(&(16 * 2048u32).to_le_bytes());
        img[s + 20..s + 24].copy_from_slice(&257u32.to_le_bytes());
        tag(&mut img[s..s + BLOCK], TAG_ANCHOR);
        let s = 257 * BLOCK;
        img[s + 188..s + 192].copy_from_slice(&(P as u32).to_le_bytes());
        tag(&mut img[s..s + BLOCK], TAG_PARTITION);
        let s = 258 * BLOCK;
        img[s + 212..s + 216].copy_from_slice(&2048u32.to_le_bytes());
        img[s + 268..s + 272].copy_from_slice(&1u32.to_le_bytes());
        img[s + 440] = 1;
        tag(&mut img[s..s + BLOCK], TAG_LOGICAL_VOLUME);
        tag(&mut img[259 * BLOCK..260 * BLOCK], TAG_TERMINATOR);
    }

    fn file_set(img: &mut [u8], root: u32) {
        let s = block(0);
        img[s + 404..s + 408].copy_from_slice(&root.to_le_bytes());
        tag(&mut img[s..s + BLOCK], TAG_FILE_SET);
    }

    /// File entry at `entry` whose single extent of `bytes` starts at block `data`.
    fn extent_entry(img: &mut [u8], entry: usize, data: usize, bytes: &[u8], directory: bool) {
        img[block(data)..block(data) + bytes.len()].copy_from_slice(bytes);
        let s = block(entry);
        if directory {
            img[s + 27] = ICB_DIRECTORY;
        }
        img[s + 56..s + 64].copy_from_slice(&(bytes.len() as u64).to_le_bytes());
        img[s + 172..s + 176].copy_from_slice(&8u32.to_le_bytes());
        img[s + 176..s + 180].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
        img[s + 180..s + 184].copy_from_slice(&(data as u32).to_le_bytes());
        tag(&mut img[s..s + BLOCK], TAG_FILE_ENTRY);
    }

    type Dirs = std::collections::BTreeMap<String, Vec<(String, Option<usize>)>>;

    /// Each directory's children: a name and, for a file, its index in `files`.
    fn tree(files: &[(&str, &[u8])]) -> Dirs {
        let mut dirs = Dirs::new();
        dirs.insert(String::new(), Vec::new());
        for (index, (path, _)) in files.iter().enumerate() {
            let parts: Vec<&str> = path.split('/').collect();
            for depth in 0..parts.len() {
                let leaf = (depth + 1 == parts.len()).then_some(index);
                let entry = (parts[depth].to_owned(), leaf);
                let list = dirs.entry(parts[..depth].join("/")).or_default();
                if !list.contains(&entry) {
                    list.push(entry);
                }
            }
        }
        dirs
    }

    /// Files given by path, one extent each.
    #[must_use]
    pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
        let dirs = tree(files);
        // Block 0: file set; then each directory's entry and data; then the files.
        let mut next = 1;
        let mut dir_blocks = std::collections::BTreeMap::new();
        for (path, list) in &dirs {
            dir_blocks.insert(path.clone(), next);
            next += 1 + ((list.len() + 1) * 64 * 8).div_ceil(BLOCK);
        }
        let mut file_blocks = Vec::new();
        for (_, data) in files {
            file_blocks.push(next);
            next += 1 + data.len().div_ceil(BLOCK).max(1);
        }
        let mut img = vec![0u8; (P + next + 1) * BLOCK];
        // Volume recognition: what image analysis checks for ISO and UDF.
        for (sector, id, kind) in [(16, b"CD001", 1u8), (17, b"BEA01", 0), (18, b"NSR02", 0)] {
            let s = sector * BLOCK;
            img[s] = kind;
            img[s + 1..s + 6].copy_from_slice(id);
            img[s + 6] = 1;
        }
        volume_descriptors(&mut img);
        file_set(&mut img, dir_blocks[""] as u32);
        for (path, list) in &dirs {
            let entry = dir_blocks[path];
            let mut records = vec![fid(FID_PARENT | FID_DIRECTORY, "", entry as u32)];
            for (name, file) in list {
                records.push(match file {
                    Some(index) => fid(0, name, file_blocks[*index] as u32),
                    None if path.is_empty() => fid(FID_DIRECTORY, name, dir_blocks[name] as u32),
                    None => fid(
                        FID_DIRECTORY,
                        name,
                        dir_blocks[&format!("{path}/{name}")] as u32,
                    ),
                });
            }
            extent_entry(&mut img, entry, entry + 1, &records.concat(), true);
        }
        for ((_, data), first) in files.iter().zip(&file_blocks) {
            extent_entry(&mut img, *first, first + 1, data, false);
        }
        img
    }

    #[must_use]
    pub fn image() -> Vec<u8> {
        let mut img = vec![0u8; 700 * BLOCK];
        volume_descriptors(&mut img);
        file_set(&mut img, 1);
        let root = [
            fid(FID_PARENT | FID_DIRECTORY, "", 1),
            fid(FID_DIRECTORY, "SOURCES", 3),
            fid(0, "readme.txt", 5),
            fid(FID_DIRECTORY, "vide", 6),
            fid(FID_DELETED, "efface.txt", 5),
        ];
        extent_entry(&mut img, 1, 2, &root.concat(), true);
        let sources = [
            fid(FID_PARENT | FID_DIRECTORY, "", 1),
            fid(0, "boot.wim", 8),
        ];
        extent_entry(&mut img, 3, 4, &sources.concat(), true);
        let s = block(5);
        img[s + 34] = 3;
        img[s + 56..s + 64].copy_from_slice(&5u64.to_le_bytes());
        img[s + 172..s + 176].copy_from_slice(&5u32.to_le_bytes());
        img[s + 176..s + 181].copy_from_slice(b"salut");
        tag(&mut img[s..s + BLOCK], TAG_FILE_ENTRY);
        extent_entry(
            &mut img,
            6,
            7,
            &fid(FID_PARENT | FID_DIRECTORY, "", 1),
            true,
        );
        // 3000-byte file in two separate extents, through an extended entry.
        let s = block(8);
        img[s + 56..s + 64].copy_from_slice(&3000u64.to_le_bytes());
        img[s + 212..s + 216].copy_from_slice(&16u32.to_le_bytes());
        img[s + 216..s + 220].copy_from_slice(&2048u32.to_le_bytes());
        img[s + 220..s + 224].copy_from_slice(&20u32.to_le_bytes());
        img[s + 224..s + 228].copy_from_slice(&952u32.to_le_bytes());
        img[s + 228..s + 232].copy_from_slice(&30u32.to_le_bytes());
        tag(&mut img[s..s + BLOCK], TAG_EXTENDED_FILE_ENTRY);
        img[block(20)..block(20) + 2048].fill(0xAA);
        img[block(30)..block(30) + 952].fill(0xBB);
        img
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn names(tree: &Tree) -> Vec<String> {
        tree.files.iter().map(|f| f.path.join("/")).collect()
    }

    #[test]
    fn l_arbre_se_lit_avec_ses_trois_formes_de_contenu() {
        let image = testing::image();
        let udf = Udf::open(image.as_slice()).unwrap();
        let tree = udf.tree().unwrap();
        assert_eq!(names(&tree), ["SOURCES/boot.wim", "readme.txt"]);
        assert_eq!(
            tree.directories,
            [vec!["SOURCES".to_owned()], vec!["vide".to_owned()]]
        );

        let inline = tree.find(&["README.TXT"]).unwrap();
        assert_eq!(inline.size, 5);
        let mut buf = [0; 5];
        read_content(image.as_slice(), &inline.content, 0, &mut buf).unwrap();
        assert_eq!(&buf, b"salut");

        let parts = tree.find(&["sources", "boot.wim"]).unwrap();
        assert_eq!(parts.size, 3000);
        let mut buf = vec![0; 3000];
        read_content(image.as_slice(), &parts.content, 0, &mut buf).unwrap();
        assert!(buf[..2048].iter().all(|b| *b == 0xAA));
        assert!(buf[2048..].iter().all(|b| *b == 0xBB));
        // Across both extents.
        let mut buf = [0; 4];
        read_content(image.as_slice(), &parts.content, 2046, &mut buf).unwrap();
        assert_eq!(buf, [0xAA, 0xAA, 0xBB, 0xBB]);
    }

    #[test]
    fn sans_ancre_ce_n_est_pas_de_l_udf() {
        let image = vec![0u8; 1 << 20];
        assert!(matches!(Udf::open(image.as_slice()), Err(UdfError::NotUdf)));
        assert!(matches!(Udf::open([].as_slice()), Err(UdfError::NotUdf)));
    }

    #[test]
    fn un_nom_ne_peut_pas_sortir_de_l_arbre() {
        for raw in [
            &b"\x08.."[..],
            b"\x08a/b",
            b"\x08a\\b",
            b"\x08",
            b"",
            b"\x07abc",
            b"\x10abc",
        ] {
            assert!(decode_name(raw).is_err(), "{raw:?}");
        }
        assert_eq!(decode_name(b"\x08caf\xE9").unwrap(), "café");
        assert_eq!(decode_name(b"\x10\x00a\x00b").unwrap(), "ab");
    }

    /// Each structure byte damaged in turn: never a panic or an endless loop.
    #[test]
    fn une_image_abimee_ne_fait_jamais_paniquer() {
        let good = testing::image();
        let ranges = [
            256 * 2048..256 * 2048 + 32,
            258 * 2048 + 200..258 * 2048 + 460,
            300 * 2048..309 * 2048 + 240,
        ];
        for range in ranges {
            for at in range.step_by(3) {
                let mut bad = good.clone();
                bad[at] ^= 0xFF;
                if let Ok(udf) = Udf::open(bad.as_slice()) {
                    let _ = udf.tree();
                }
            }
        }
    }
}
