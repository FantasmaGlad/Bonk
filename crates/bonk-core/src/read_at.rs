use std::io;

/// Positional reads: several readers may share the image, without a shared cursor.
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
