use std::cell::RefCell;

use super::MAX_TRANSFER;
use super::session::{Inquiry, MassStorage};
use super::transport::{BulkTransport, MscError};
use crate::block::{BlockDevice, BlockError, check_access, check_geometry};

pub struct UsbDisk<T> {
    pub(super) session: RefCell<MassStorage<T>>,
    size: u64,
    block_size: u32,
}

impl<T: BulkTransport> UsbDisk<T> {
    /// Waits for the key and reads its capacity.
    ///
    /// # Errors
    /// No answer, or invalid geometry.
    pub fn open(transport: T) -> Result<Self, BlockError> {
        let mut session = MassStorage::new(transport);
        session.wait_ready()?;
        let (blocks, block_size) = session.capacity()?;
        let size =
            blocks
                .checked_mul(u64::from(block_size))
                .ok_or(BlockError::InvalidGeometry {
                    size: u64::MAX,
                    block_size,
                })?;
        check_geometry(size, block_size)?;
        Ok(Self {
            session: RefCell::new(session),
            size,
            block_size,
        })
    }

    /// Vendor and product as the drive names itself.
    ///
    /// # Errors
    /// No answer, or a short reply.
    pub fn inquiry(&self) -> Result<Inquiry, MscError> {
        self.session.borrow_mut().inquiry()
    }
}

impl<T: BulkTransport> BlockDevice for UsbDisk<T> {
    fn size(&self) -> u64 {
        self.size
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, buf.len())?;
        let block = u64::from(self.block_size);
        let chunk = MAX_TRANSFER - MAX_TRANSFER % self.block_size as usize;
        let mut session = self.session.borrow_mut();
        let mut lba = offset / block;
        for part in buf.chunks_mut(chunk) {
            session.read_blocks(lba, part, self.block_size)?;
            lba += part.len() as u64 / block;
        }
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, data.len())?;
        let block = u64::from(self.block_size);
        let chunk = MAX_TRANSFER - MAX_TRANSFER % self.block_size as usize;
        let session = self.session.get_mut();
        let mut lba = offset / block;
        for part in data.chunks(chunk) {
            session.write_blocks(lba, part, self.block_size)?;
            lba += part.len() as u64 / block;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        self.session.get_mut().synchronize()?;
        Ok(())
    }
}
