use std::io::{self, Read, Seek, SeekFrom};

use crate::bytes::get_u32;

const SECTOR: u64 = 2048;
const FIRST_DESCRIPTOR: u64 = 16;
/// Bounds what a damaged or hostile image makes Bonk read for one directory.
const MAX_DIRECTORY_BYTES: u32 = 1 << 20;
const FLAG_DIRECTORY: u8 = 2;

/// An ISO 9660 image, read only as far as finding a file by its path: what tells a live system's family.
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
    // Absent when the record ends with its name.
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

    /// Size in bytes of the file at `path` (`/` separated, from the root); `None` when absent or a directory.
    ///
    /// # Errors
    /// Unreadable or malformed directories.
    pub fn file_size(&mut self, path: &str) -> io::Result<Option<u64>> {
        let mut current = self.root;
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        for (index, part) in parts.iter().enumerate() {
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
            if index + 1 == parts.len() {
                return Ok((!current.directory).then_some(u64::from(current.size)));
            }
        }
        Ok(None)
    }
}

/// Test images: a root, directories and empty files, with Rock Ridge names, as `xorriso -R` lays them out.
#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::*;

    fn record(name: &str, extent: u32, size: u32, directory: bool) -> Vec<u8> {
        let short = name.to_ascii_uppercase();
        let short = if directory {
            short
        } else {
            format!("{short};1")
        };
        let nm = [
            b"NM".as_slice(),
            &[5 + name.len() as u8, 1, 0],
            name.as_bytes(),
        ]
        .concat();
        let mut r = vec![0u8; 33];
        r.extend(short.as_bytes());
        if short.len() % 2 == 0 {
            r.push(0);
        }
        r.extend(nm);
        r[0] = r.len() as u8;
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[10..14].copy_from_slice(&size.to_le_bytes());
        r[25] = if directory { FLAG_DIRECTORY } else { 0 };
        r[32] = short.len() as u8;
        r
    }

    fn dot(extent: u32, name: u8) -> Vec<u8> {
        let mut r = vec![0u8; 34];
        r[0] = 34;
        r[2..6].copy_from_slice(&extent.to_le_bytes());
        r[10..14].copy_from_slice(&(SECTOR as u32).to_le_bytes());
        r[25] = FLAG_DIRECTORY;
        r[32] = 1;
        r[33] = name;
        r
    }

    /// One sector per directory, files as given (path, size); data is never read.
    #[must_use]
    pub fn image(label: &str, files: &[(&str, u32)]) -> Vec<u8> {
        let mut directories: Vec<String> = vec![String::new()];
        for (path, _) in files {
            let mut prefix = String::new();
            for part in path
                .split('/')
                .rev()
                .skip(1)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                prefix = if prefix.is_empty() {
                    part.to_owned()
                } else {
                    format!("{prefix}/{part}")
                };
                if !directories.contains(&prefix) {
                    directories.push(prefix.clone());
                }
            }
        }
        let first = 20u32;
        let extent =
            |dir: &str| first + directories.iter().position(|d| d == dir).unwrap_or(0) as u32;
        let mut image = vec![0u8; (first as usize + directories.len()) * SECTOR as usize];
        for dir in &directories {
            let mut sector = [dot(extent(dir), 0), dot(extent(dir), 1)].concat();
            let parent_of = |path: &str| path.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
            for child in directories
                .iter()
                .filter(|d| !d.is_empty() && parent_of(d) == *dir)
            {
                let name = child.rsplit('/').next().unwrap_or(child);
                sector.extend(record(name, extent(child), SECTOR as u32, true));
            }
            for (path, size) in files.iter().filter(|(p, _)| parent_of(p) == *dir) {
                let name = path.rsplit('/').next().unwrap_or(path);
                sector.extend(record(name, 0, *size, false));
            }
            let at = extent(dir) as usize * SECTOR as usize;
            image[at..at + sector.len()].copy_from_slice(&sector);
        }
        let pvd = &mut image[(FIRST_DESCRIPTOR * SECTOR) as usize..][..SECTOR as usize];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[40..72].fill(b' ');
        pvd[40..40 + label.len()].copy_from_slice(label.as_bytes());
        pvd[156..190].copy_from_slice(&dot(first, 0));
        image
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    #[test]
    fn fichiers_trouves_par_leur_nom_rock_ridge() -> io::Result<()> {
        let image = testing::image(
            "Fedora-WS-Live-44",
            &[
                ("boot/x86_64/loader/linux", 18_479_464),
                ("boot/x86_64/loader/initrd", 1000),
                ("EFI/BOOT/grub.cfg", 111),
            ],
        );
        let mut iso = Iso::open(Cursor::new(image))?;
        assert_eq!(iso.label(), "Fedora-WS-Live-44");
        assert_eq!(
            iso.file_size("/boot/x86_64/loader/linux")?,
            Some(18_479_464)
        );
        assert_eq!(iso.file_size("EFI/BOOT/GRUB.CFG")?, Some(111));
        assert_eq!(iso.file_size("boot/x86_64/loader")?, None);
        assert_eq!(iso.file_size("boot/x86_64/loader/absent")?, None);
        assert_eq!(iso.file_size("boot/x86_64/loader/linux/plus")?, None);
        Ok(())
    }

    #[test]
    fn une_image_qui_n_est_pas_iso_est_refusee() {
        assert!(Iso::open(Cursor::new(vec![0u8; 40 * 2048])).is_err());
        assert!(Iso::open(Cursor::new(vec![0u8; 100])).is_err());
    }

    #[test]
    fn un_repertoire_demesure_est_refuse_sans_allocation() -> io::Result<()> {
        let mut image = testing::image("X", &[("a/b", 1)]);
        // Root directory size field of the volume descriptor.
        let at = (FIRST_DESCRIPTOR * SECTOR) as usize + 156 + 10;
        image[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut iso = Iso::open(Cursor::new(image))?;
        assert!(iso.file_size("a/b").is_err());
        Ok(())
    }

    #[test]
    fn nom_iso_sans_rock_ridge() {
        let mut record = vec![0u8; 33];
        record.extend(b"VMLINUZ.;1");
        record[32] = 10;
        assert_eq!(name_of(&record).as_deref(), Some("VMLINUZ"));
    }
}
