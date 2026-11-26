use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use bonk_core::codec::{self, CodecError, Source};
use bonk_core::read_at::ReadAt;

use super::types::{JobError, SourceInfo, fail};
use crate::errors::Code;

pub struct ImageFile {
    source: Backing,
    path: String,
    len: u64,
}

#[derive(Clone)]
enum Backing {
    File(Arc<File>),
    /// Read by position only, as a file on a server.
    At(Arc<dyn ReadAt + Send + Sync>),
}

impl Backing {
    fn read_at(&self, buf: &mut [u8], offset: u64, len: u64) -> io::Result<usize> {
        match self {
            Self::File(file) => bonk_hal::image::read_at(file, buf, offset),
            Self::At(source) => {
                let left = len.saturating_sub(offset);
                let n = usize::try_from(left).map_or(buf.len(), |left| left.min(buf.len()));
                source.read_exact_at(offset, &mut buf[..n])?;
                Ok(n)
            }
        }
    }
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
            source: Backing::File(Arc::new(file)),
            path: path.display().to_string(),
            len: metadata.len(),
        })
    }

    /// An image read by position, `name` standing for its path in reports.
    pub fn at(name: String, len: u64, source: Arc<dyn ReadAt + Send + Sync>) -> Self {
        Self {
            source: Backing::At(source),
            path: name,
            len,
        }
    }

    pub fn size(&self) -> u64 {
        self.len
    }

    pub fn raw(&self) -> FileReader {
        FileReader {
            source: self.source.clone(),
            len: self.len,
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
    source: Backing,
    len: u64,
    position: u64,
}

impl ReadAt for ImageFile {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        match &self.source {
            Backing::File(file) => bonk_hal::image::read_exact_at(file, buf, offset),
            Backing::At(source) => source.read_exact_at(offset, buf),
        }
    }
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.source.read_at(buf, self.position, self.len)?;
        self.position += n as u64;
        Ok(n)
    }
}
