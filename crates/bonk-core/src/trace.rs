//! Trace des accès bloc (`--trace-io`) : décalage, longueur, durée et somme de contrôle de chaque
//! accès, **jamais les données**, qu'un disque réel peut porter de personnelles. Le début des données
//! n'est joint que sur demande explicite, pour un fichier image.

use std::time::Instant;

use crate::block::{BlockDevice, BlockError};
use crate::partition::crc32;

pub struct TracedDevice {
    inner: Box<dyn BlockDevice>,
    include_head: bool,
}

impl TracedDevice {
    /// `include_head` ajoute les 16 premiers octets de chaque accès : réservé aux images.
    #[must_use]
    pub fn new(inner: Box<dyn BlockDevice>, include_head: bool) -> Self {
        Self {
            inner,
            include_head,
        }
    }

    fn note(&self, op: &str, offset: u64, data: &[u8], started: Instant, ok: bool) {
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let crc = format!("{:08x}", crc32(data));
        let head: String = if self.include_head {
            data.iter().take(16).map(|b| format!("{b:02x}")).collect()
        } else {
            String::new()
        };
        if ok {
            log::trace!(
                target: "bonk_core::trace",
                op = op, offset = offset, len = data.len(), micros = micros, crc32 = crc.as_str(), head = head.as_str();
                "block access"
            );
        } else {
            log::debug!(
                target: "bonk_core::trace",
                op = op, offset = offset, len = data.len(), micros = micros;
                "block access failed"
            );
        }
    }
}

impl BlockDevice for TracedDevice {
    fn size(&self) -> u64 {
        self.inner.size()
    }

    fn block_size(&self) -> u32 {
        self.inner.block_size()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        let started = Instant::now();
        let result = self.inner.read_at(offset, buf);
        self.note("read", offset, buf, started, result.is_ok());
        result
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        let started = Instant::now();
        let result = self.inner.write_at(offset, data);
        self.note("write", offset, data, started, result.is_ok());
        result
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        let started = Instant::now();
        let result = self.inner.flush();
        self.note("flush", 0, &[], started, result.is_ok());
        result
    }

    fn drop_cache(&mut self) -> Result<(), BlockError> {
        let started = Instant::now();
        let result = self.inner.drop_cache();
        self.note("drop_cache", 0, &[], started, result.is_ok());
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::MemoryDevice;

    #[test]
    fn l_enveloppe_est_transparente() -> Result<(), BlockError> {
        let mut dev = TracedDevice::new(Box::new(MemoryDevice::new(4 * 512, 512)?), false);
        assert_eq!((dev.size(), dev.block_size()), (2048, 512));
        dev.write_at(512, &[7; 512])?;
        let mut back = [0u8; 512];
        dev.read_at(512, &mut back)?;
        assert_eq!(back, [7; 512]);
        dev.flush()?;
        assert!(matches!(
            dev.write_at(7, &[0; 512]),
            Err(BlockError::Unaligned { .. })
        ));
        Ok(())
    }
}
