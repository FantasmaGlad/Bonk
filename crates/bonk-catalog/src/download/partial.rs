use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// `bytes 100-199/200`; the total may be `*`.
pub(super) fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.parse().ok()?, total.parse().ok()))
}

/// `bytes */200` in a 416 response.
pub(super) fn unsatisfied_total(value: &str) -> Option<u64> {
    value.strip_prefix("bytes */")?.parse().ok()
}

pub(super) fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub(super) struct Partial {
    pub(super) file: File,
    pub(super) done: u64,
    pub(super) validator_path: PathBuf,
    pub(super) validator: Option<String>,
}

impl Partial {
    pub(super) fn open(part: &Path) -> io::Result<Self> {
        // Not in append mode: Windows would not allow truncating the file.
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(part)?;
        let done = file.seek(SeekFrom::End(0))?;
        let validator_path = with_suffix(part, ".validateur");
        let validator = fs::read_to_string(&validator_path).ok();
        Ok(Self {
            file,
            done,
            validator_path,
            validator,
        })
    }

    pub(super) fn restart(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.done = 0;
        self.validator = None;
        match fs::remove_file(&self.validator_path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    pub(super) fn remember(&mut self, validator: Option<String>) -> io::Result<()> {
        if let Some(validator) = &validator {
            fs::write(&self.validator_path, validator)?;
        }
        self.validator = validator;
        Ok(())
    }

    pub(super) fn finish(self, part: &Path, dest: &Path) -> io::Result<()> {
        self.file.sync_all()?;
        // Closed before renaming: Windows refuses to rename an open file.
        drop(self.file);
        fs::rename(part, dest)?;
        match fs::remove_file(&self.validator_path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }
}
