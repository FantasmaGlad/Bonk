use std::io;

use crate::block::BlockDevice;
use crate::partition::read_range;

pub trait ReadAt {
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

pub struct OnDevice<'a>(pub &'a dyn BlockDevice);

impl ReadAt for OnDevice<'_> {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let bytes = read_range(self.0, offset, buf.len() as u64).map_err(io::Error::other)?;
        buf.copy_from_slice(&bytes);
        Ok(())
    }
}

/// `Read` and `Seek` over positional reads, for readers written against a stream.
pub struct Cursor<'a, S: ReadAt + ?Sized> {
    source: &'a S,
    len: u64,
    position: u64,
}

impl<'a, S: ReadAt + ?Sized> Cursor<'a, S> {
    #[must_use]
    pub fn new(source: &'a S, len: u64) -> Self {
        Self {
            source,
            len,
            position: 0,
        }
    }
}

impl<S: ReadAt + ?Sized> io::Read for Cursor<'_, S> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.position);
        let n = usize::try_from(left).map_or(buf.len(), |left| left.min(buf.len()));
        self.source.read_exact_at(self.position, &mut buf[..n])?;
        self.position += n as u64;
        Ok(n)
    }
}

impl<S: ReadAt + ?Sized> io::Seek for Cursor<'_, S> {
    fn seek(&mut self, to: io::SeekFrom) -> io::Result<u64> {
        let target = match to {
            io::SeekFrom::Start(offset) => Some(offset),
            io::SeekFrom::End(delta) => self.len.checked_add_signed(delta),
            io::SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        };
        self.position = target.ok_or(io::ErrorKind::InvalidInput)?;
        Ok(self.position)
    }
}
