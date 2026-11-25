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
