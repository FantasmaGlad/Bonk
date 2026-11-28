use super::*;

#[derive(Clone)]
pub(super) struct Shared(pub(super) Rc<RefCell<MemoryDevice>>);

impl Shared {
    pub(super) fn new() -> Self {
        Self::sized(DISK_SIZE as u64)
    }

    pub(super) fn sized(size: u64) -> Self {
        Self(Rc::new(RefCell::new(
            MemoryDevice::new(size, 512).expect("géométrie valide"),
        )))
    }
}

impl BlockDevice for Shared {
    fn size(&self) -> u64 {
        self.0.borrow().size()
    }
    fn block_size(&self) -> u32 {
        512
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.0.borrow().read_at(offset, buf)
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        self.0.borrow_mut().write_at(offset, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

/// Logs the cache drop and flags any read before it: on a real disk, a check read before the drop reads the
/// kernel cache, not the drive.
pub(super) struct Logged {
    pub(super) inner: Box<dyn BlockDevice>,
    pub(super) log: Log,
    pub(super) cache_clean: Cell<bool>,
    pub(super) open_devices: Rc<Cell<usize>>,
}

impl Drop for Logged {
    fn drop(&mut self) {
        self.open_devices.set(self.open_devices.get() - 1);
    }
}

impl BlockDevice for Logged {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn block_size(&self) -> u32 {
        self.inner.block_size()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if !self.cache_clean.get() {
            self.log
                .borrow_mut()
                .push("lecture_avant_vidage_du_cache".to_owned());
        }
        self.inner.read_at(offset, buf)
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        self.cache_clean.set(false);
        self.inner.write_at(offset, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
    fn drop_cache(&mut self) -> Result<(), BlockError> {
        self.cache_clean.set(true);
        self.log.borrow_mut().push("drop_cache".to_owned());
        self.inner.drop_cache()
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct Faults {
    pub(super) reads: bool,
    pub(super) first_read: bool,
    pub(super) flush: bool,
    pub(super) unplugged_after: Option<u64>,
    pub(super) lost_after: Option<u64>,
}

pub(super) struct Faulty {
    pub(super) inner: Shared,
    pub(super) faults: Faults,
    pub(super) written: u64,
    pub(super) first_read_done: Cell<bool>,
}

impl BlockDevice for Faulty {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn block_size(&self) -> u32 {
        512
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_at(offset, buf)?;
        let first = self.faults.first_read && offset == 0 && !self.first_read_done.replace(true);
        if (self.faults.reads || first) && offset == 0 {
            buf[0] ^= 0xFF;
        }
        Ok(())
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        if self
            .faults
            .unplugged_after
            .is_some_and(|limit| self.written >= limit)
        {
            return Err(BlockError::Io(io::Error::new(
                io::ErrorKind::NotConnected,
                "device disconnected",
            )));
        }
        self.written += data.len() as u64;
        if self.faults.lost_after.is_some_and(|limit| offset >= limit) {
            return Ok(());
        }
        self.inner.write_at(offset, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        if self.faults.flush {
            return Err(BlockError::Io(io::Error::other(
                "SYNCHRONIZE CACHE refused",
            )));
        }
        Ok(())
    }
}
