use std::io::{self, Read, Seek, SeekFrom};

use crate::bytes::get_u32;

const SECTOR: u64 = 2048;
const FIRST_DESCRIPTOR: u64 = 16;
/// Bounds what a damaged or hostile image makes Bonk read for one directory.
const MAX_DIRECTORY_BYTES: u32 = 1 << 20;
const FLAG_DIRECTORY: u8 = 2;

pub struct Iso<R> {
    reader: R,
    label: String,
    root: Record,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Record {
    extent: u32,
    size: u32,
    directory: bool,
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// The Rock Ridge name (`NM`) when present, else the ISO 9660 name without its version, compared without case.
fn name_of(record: &[u8]) -> Option<String> {
    let length = usize::from(*record.get(32)?);
    let raw = record.get(33..33 + length)?;
    let mut rest = record
        .get(33 + length + (1 - length % 2)..)
        .unwrap_or_default();
    while let [a, b, size, ..] = *rest {
        let size = usize::from(size);
        if size < 4 || size > rest.len() {
            break;
        }
        if [a, b] == *b"NM" && size > 5 {
            return Some(String::from_utf8_lossy(&rest[5..size]).into_owned());
        }
        rest = &rest[size..];
    }
    let text = String::from_utf8_lossy(raw);
    let text = text.split(';').next().unwrap_or_default();
    Some(text.strip_suffix('.').unwrap_or(text).to_owned())
}

fn record_of(record: &[u8]) -> Option<Record> {
    Some(Record {
        extent: get_u32(record, 2)?,
        size: get_u32(record, 10)?,
        directory: record.get(25)? & FLAG_DIRECTORY != 0,
    })
}

impl<R: Read + Seek> Iso<R> {
    /// # Errors
    /// Not an ISO 9660 image, or unreadable.
    pub fn open(mut reader: R) -> io::Result<Self> {
        let mut descriptor = [0u8; SECTOR as usize];
        reader.seek(SeekFrom::Start(FIRST_DESCRIPTOR * SECTOR))?;
        reader.read_exact(&mut descriptor)?;
        if descriptor[0] != 1 || &descriptor[1..6] != b"CD001" {
            return Err(invalid("not an ISO 9660 image"));
        }
        let root = record_of(&descriptor[156..190]).ok_or_else(|| invalid("root directory"))?;
        let label = String::from_utf8_lossy(&descriptor[40..72])
            .trim()
            .to_owned();
        Ok(Self {
            reader,
            label,
            root,
        })
    }

    /// The volume label, as `probe --label` gives it to GRUB.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    fn children(&mut self, directory: Record) -> io::Result<Vec<(String, Record)>> {
        if directory.size > MAX_DIRECTORY_BYTES {
            return Err(invalid("directory larger than any live image has"));
        }
        let mut data = vec![0u8; directory.size as usize];
        self.reader
            .seek(SeekFrom::Start(u64::from(directory.extent) * SECTOR))?;
        self.reader.read_exact(&mut data)?;
        let mut found = Vec::new();
        let mut at = 0usize;
        while at < data.len() {
            let length = usize::from(data[at]);
            if length == 0 {
                // Records never cross a sector: the rest of this one is padding.
                at = (at / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            let record = data
                .get(at..at + length)
                .ok_or_else(|| invalid("directory record"))?;
            // `.` and `..` are the one-byte names 0 and 1.
            let dot = record.get(32) == Some(&1) && matches!(record.get(33), Some(0 | 1));
            if !dot && let (Some(name), Some(entry)) = (name_of(record), record_of(record)) {
                found.push((name, entry));
            }
            at += length;
        }
        Ok(found)
    }

    fn record_at(&mut self, path: &str) -> io::Result<Option<Record>> {
        let mut current = self.root;
        for part in path.split('/').filter(|part| !part.is_empty()) {
            if !current.directory {
                return Ok(None);
            }
            let Some((_, next)) = self
                .children(current)?
                .into_iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(part))
            else {
                return Ok(None);
            };
            current = next;
        }
        Ok(Some(current))
    }

    /// Size in bytes of the file at `path` (`/` separated, from the root); `None` when absent or a directory.
    ///
    /// # Errors
    /// Unreadable or malformed directories.
    pub fn file_size(&mut self, path: &str) -> io::Result<Option<u64>> {
        Ok(self
            .record_at(path)?
            .filter(|record| !record.directory)
            .map(|record| u64::from(record.size)))
    }

    /// Paths of the files matching `pattern`, whose last part may hold one `*`, as GRUB expands it.
    ///
    /// # Errors
    /// Unreadable or malformed directories.
    pub fn matching(&mut self, pattern: &str) -> io::Result<Vec<String>> {
        let (folder, name) = pattern.rsplit_once('/').unwrap_or(("", pattern));
        let Some(directory) = self.record_at(folder)?.filter(|record| record.directory) else {
            return Ok(Vec::new());
        };
        Ok(self
            .children(directory)?
            .into_iter()
            .filter(|(child, record)| !record.directory && name_matches(name, child))
            .map(|(child, _)| match folder {
                "" => child,
                _ => format!("{folder}/{child}"),
            })
            .collect())
    }
}

fn name_matches(pattern: &str, name: &str) -> bool {
    let (pattern, name) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    match pattern.split_once('*') {
        Some((start, end)) => {
            name.len() >= start.len() + end.len() && name.starts_with(start) && name.ends_with(end)
        }
        None => pattern == name,
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
