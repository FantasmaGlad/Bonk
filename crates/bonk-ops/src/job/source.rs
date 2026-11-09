use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use bonk_core::codec::{self, CodecError, Source};

use super::types::{JobError, SourceInfo, fail};
use crate::errors::Code;

/// One descriptor, as many independent reads as steps.
pub struct ImageFile {
    file: Arc<File>,
    path: String,
    len: u64,
}

impl ImageFile {
    pub fn open(path: &Path) -> Result<Self, JobError> {
        let unreadable =
            |e: io::Error| fail(Code::SourceUnreadable, format!("{}: {e}", path.display()));
        let file = File::open(path).map_err(unreadable)?;
        // Checked on the open descriptor, not the path, which may have changed.
        let metadata = file.metadata().map_err(unreadable)?;
        if !metadata.is_file() {
            return Err(fail(
                Code::SourceUnreadable,
                format!("{} is not a regular file", path.display()),
            ));
        }
        Ok(Self {
            file: Arc::new(file),
            path: path.display().to_string(),
            len: metadata.len(),
        })
    }

    /// Without moving any reader.
    pub(super) fn read_exact_positioned(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        bonk_hal::image::read_exact_at(&self.file, buf, offset)
    }

    pub fn size(&self) -> u64 {
        self.len
    }

    pub fn raw(&self) -> FileReader {
        FileReader {
            file: Arc::clone(&self.file),
            position: 0,
        }
    }

    pub(crate) fn decoded(&self) -> Result<(Source, SourceInfo), JobError> {
        let source = codec::open(self.raw(), Some(self.len)).map_err(|e| match e {
            CodecError::Unsupported(_) => fail(Code::SourceFormatUnsupported, e),
            CodecError::Io(_) => fail(Code::SourceUnreadable, format!("{}: {e}", self.path)),
        })?;
        let info = SourceInfo {
            path: self.path.clone(),
            format: source.format,
            size_bytes: source.output_len,
        };
        Ok((source, info))
    }
}

/// Sequential reads through positional access: readers of the same file share no cursor.
pub struct FileReader {
    file: Arc<File>,
    position: u64,
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = bonk_hal::image::read_at(&self.file, buf, self.position)?;
        self.position += n as u64;
        Ok(n)
    }
}
