use super::*;

enum Data<'a> {
    None,
    In(&'a mut [u8]),
    Out(&'a [u8]),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inquiry {
    pub vendor: String,
    pub product: String,
}

pub struct MassStorage<T> {
    pub(super) transport: T,
    tag: u32,
}

impl<T: BulkTransport> MassStorage<T> {
    pub fn new(transport: T) -> Self {
        Self { transport, tag: 0 }
    }

    /// Returns the bytes transferred. `Check` does not carry the sense yet.
    fn execute(&mut self, cdb: &[u8], data: Data<'_>) -> Result<usize, MscError> {
        self.tag = self.tag.wrapping_add(1);
        let tag = self.tag;
        let (length, flags) = match &data {
            Data::None => (0, 0),
            Data::In(buf) => (buf.len(), 0x80),
            Data::Out(buf) => (buf.len(), 0),
        };
        let mut cbw = [0u8; CBW_LEN];
        cbw[0..4].copy_from_slice(&CBW_SIGNATURE.to_le_bytes());
        cbw[4..8].copy_from_slice(&tag.to_le_bytes());
        cbw[8..12].copy_from_slice(&u32::try_from(length).unwrap_or(u32::MAX).to_le_bytes());
        cbw[12] = flags;
        cbw[14] = u8::try_from(cdb.len()).unwrap_or(16);
        cbw[15..15 + cdb.len()].copy_from_slice(cdb);
        if let Err(error) = self.transport.send(&cbw) {
            if matches!(error, BulkError::Stall) {
                self.transport.reset()?;
            }
            return Err(error.into());
        }

        let done = self.transfer(data)?;
        let csw = self.read_status()?;
        self.check_status(&csw, tag)?;
        // Trust the device residue too: a device may pad with zeros.
        let residue = u32::from_le_bytes([csw[8], csw[9], csw[10], csw[11]]) as usize;
        Ok(done.min(length.saturating_sub(residue)))
    }

    fn transfer(&mut self, data: Data<'_>) -> Result<usize, MscError> {
        match data {
            Data::None => Ok(0),
            Data::In(buf) => self.receive_all(buf),
            Data::Out(buf) => match self.transport.send(buf) {
                Ok(()) => Ok(buf.len()),
                Err(BulkError::Stall) => {
                    self.transport.clear_halt(Direction::Out)?;
                    Ok(0)
                }
                Err(error) => Err(error.into()),
            },
        }
    }

    fn receive_all(&mut self, buf: &mut [u8]) -> Result<usize, MscError> {
        let mut done = 0;
        while done < buf.len() {
            let asked = buf.len() - done;
            match self.transport.receive(&mut buf[done..]) {
                Ok(read) => {
                    done += read;
                    // Short packet: the data phase ended, the status follows.
                    if read < asked {
                        break;
                    }
                }
                Err(BulkError::Stall) => {
                    self.transport.clear_halt(Direction::In)?;
                    break;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(done)
    }

    fn check_status(&mut self, csw: &[u8; CSW_LEN], tag: u32) -> Result<(), MscError> {
        let failure = if u32::from_le_bytes([csw[0], csw[1], csw[2], csw[3]]) != CSW_SIGNATURE {
            MscError::Protocol("bad status signature")
        } else if u32::from_le_bytes([csw[4], csw[5], csw[6], csw[7]]) != tag {
            MscError::Protocol("status of another command")
        } else {
            match csw[12] {
                0 => return Ok(()),
                1 => return Err(MscError::Check(None)),
                2 => MscError::Phase,
                _ => MscError::Protocol("unknown status"),
            }
        };
        self.transport.reset()?;
        Err(failure)
    }

    /// A stall during status is cleared once (bulk-only specification); the second time, reset.
    fn read_status(&mut self) -> Result<[u8; CSW_LEN], MscError> {
        let mut csw = [0u8; CSW_LEN];
        for attempt in 0..2 {
            match self.transport.receive(&mut csw) {
                Ok(CSW_LEN) => return Ok(csw),
                Ok(_) => {
                    self.transport.reset()?;
                    return Err(MscError::Protocol("short status"));
                }
                Err(BulkError::Stall) if attempt == 0 => {
                    self.transport.clear_halt(Direction::In)?;
                }
                Err(BulkError::Stall) => {
                    self.transport.reset()?;
                    return Err(MscError::Bulk(BulkError::Stall));
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(MscError::Protocol("no status"))
    }

    /// On failure, reads the sense to report it.
    fn command(&mut self, cdb: &[u8], data: Data<'_>) -> Result<usize, MscError> {
        match self.execute(cdb, data) {
            Err(MscError::Check(_)) => Err(MscError::Check(self.request_sense().ok())),
            other => other,
        }
    }

    fn request_sense(&mut self) -> Result<Sense, MscError> {
        let mut reply = [0u8; 18];
        self.execute(&[0x03, 0, 0, 0, 18, 0], Data::In(&mut reply))?;
        Ok(Sense {
            key: reply[2] & 0x0F,
            code: reply[12],
            qualifier: reply[13],
        })
    }

    /// # Errors
    /// Command failure.
    pub fn inquiry(&mut self) -> Result<Inquiry, MscError> {
        let mut reply = [0u8; 36];
        let done = self.command(&[0x12, 0, 0, 0, 36, 0], Data::In(&mut reply))?;
        if done < 32 {
            return Err(MscError::Short { expected: 36, done });
        }
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_owned();
        Ok(Inquiry {
            vendor: text(&reply[8..16]),
            product: text(&reply[16..32]),
        })
    }

    /// A freshly plugged key first reports its media change.
    ///
    /// # Errors
    /// Still not ready after a few tries.
    pub fn wait_ready(&mut self) -> Result<(), MscError> {
        let mut last = MscError::Check(None);
        for _ in 0..READY_ATTEMPTS {
            match self.command(&[0, 0, 0, 0, 0, 0], Data::None) {
                Ok(_) => return Ok(()),
                Err(error @ MscError::Check(_)) => last = error,
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    /// Block count and block size. `READ CAPACITY(16)` only when (10) is not enough.
    ///
    /// # Errors
    /// Command failure, or short reply.
    pub fn capacity(&mut self) -> Result<(u64, u32), MscError> {
        let mut short = [0u8; 8];
        let done = self.command(&[0x25, 0, 0, 0, 0, 0, 0, 0, 0, 0], Data::In(&mut short))?;
        if done < 8 {
            return Err(MscError::Short { expected: 8, done });
        }
        let last = u32::from_be_bytes([short[0], short[1], short[2], short[3]]);
        let block = u32::from_be_bytes([short[4], short[5], short[6], short[7]]);
        if last != u32::MAX {
            return Ok((u64::from(last) + 1, block));
        }
        let mut long = [0u8; 32];
        let mut cdb = [0u8; 16];
        cdb[0] = 0x9E;
        cdb[1] = 0x10;
        cdb[13] = 32;
        let done = self.command(&cdb, Data::In(&mut long))?;
        if done < 12 {
            return Err(MscError::Short { expected: 32, done });
        }
        let mut last = [0u8; 8];
        last.copy_from_slice(&long[..8]);
        Ok((
            u64::from_be_bytes(last) + 1,
            u32::from_be_bytes([long[8], long[9], long[10], long[11]]),
        ))
    }

    /// The 10-byte form while it fits, else the 16-byte form.
    pub(super) fn rw_cdb(opcode_10: u8, opcode_16: u8, lba: u64, blocks: u32) -> Vec<u8> {
        if let (Ok(short_lba), Ok(short_blocks)) = (u32::try_from(lba), u16::try_from(blocks)) {
            let mut cdb = vec![0u8; 10];
            cdb[0] = opcode_10;
            cdb[2..6].copy_from_slice(&short_lba.to_be_bytes());
            cdb[7..9].copy_from_slice(&short_blocks.to_be_bytes());
            cdb
        } else {
            let mut cdb = vec![0u8; 16];
            cdb[0] = opcode_16;
            cdb[2..10].copy_from_slice(&lba.to_be_bytes());
            cdb[10..14].copy_from_slice(&blocks.to_be_bytes());
            cdb
        }
    }

    pub(super) fn read_blocks(
        &mut self,
        lba: u64,
        buf: &mut [u8],
        block: u32,
    ) -> Result<(), MscError> {
        let blocks = u32::try_from(buf.len() / block as usize).unwrap_or(u32::MAX);
        let cdb = Self::rw_cdb(0x28, 0x88, lba, blocks);
        let expected = buf.len();
        let done = self.command(&cdb, Data::In(buf))?;
        if done == expected {
            Ok(())
        } else {
            Err(MscError::Short { expected, done })
        }
    }

    pub(super) fn write_blocks(
        &mut self,
        lba: u64,
        data: &[u8],
        block: u32,
    ) -> Result<(), MscError> {
        let blocks = u32::try_from(data.len() / block as usize).unwrap_or(u32::MAX);
        let cdb = Self::rw_cdb(0x2A, 0x8A, lba, blocks);
        let done = self.command(&cdb, Data::Out(data))?;
        if done == data.len() {
            Ok(())
        } else {
            Err(MscError::Short {
                expected: data.len(),
                done,
            })
        }
    }

    pub(super) fn synchronize(&mut self) -> Result<(), MscError> {
        match self.command(&[0x35, 0, 0, 0, 0, 0, 0, 0, 0, 0], Data::None) {
            // Readers without a write cache reject the opcode itself (seen on a USB SD reader); the
            // read-back stays the real check. Any other refusal is an error.
            Err(MscError::Check(Some(Sense {
                key: SENSE_ILLEGAL_REQUEST,
                code: ASC_INVALID_OPCODE,
                ..
            }))) => Ok(()),
            result => result.map(|_| ()),
        }
    }
}
