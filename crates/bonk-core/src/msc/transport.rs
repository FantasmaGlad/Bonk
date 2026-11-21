use std::fmt;
use std::io;

use crate::block::BlockError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug)]
pub enum BulkError {
    Stall,
    Disconnected,
    Other(io::Error),
}

pub trait BulkTransport {
    /// # Errors
    /// Endpoint stall, unplug, or system error.
    fn send(&mut self, data: &[u8]) -> Result<(), BulkError>;
    /// Receives at most `buf.len()` bytes. Fewer means the device ended the transfer (short packet): a
    /// transport that splits transfers chains them until `buf` is full or a short packet arrives.
    ///
    /// # Errors
    /// Endpoint stall, unplug, or system error.
    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, BulkError>;
    /// # Errors
    /// Unplug or system error.
    fn clear_halt(&mut self, direction: Direction) -> Result<(), BulkError>;
    /// Mass storage reset: class request, then both halts cleared.
    ///
    /// # Errors
    /// Unplug or system error.
    fn reset(&mut self) -> Result<(), BulkError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sense {
    pub key: u8,
    pub code: u8,
    pub qualifier: u8,
}

#[derive(Debug)]
pub enum MscError {
    Bulk(BulkError),
    Protocol(&'static str),
    /// The exchange is lost; the device was reset.
    Phase,
    /// `None` when the sense could not be read.
    Check(Option<Sense>),
    Short {
        expected: usize,
        done: usize,
    },
}

impl fmt::Display for MscError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bulk(BulkError::Stall) => f.write_str("USB endpoint stalled"),
            Self::Bulk(BulkError::Disconnected) => f.write_str("USB device disconnected"),
            Self::Bulk(BulkError::Other(err)) => write!(f, "USB transfer failed: {err}"),
            Self::Protocol(what) => write!(f, "USB storage protocol error: {what}"),
            Self::Phase => f.write_str("USB storage phase error"),
            Self::Check(Some(sense)) => write!(
                f,
                "SCSI command failed: sense {:02x}/{:02x}/{:02x}",
                sense.key, sense.code, sense.qualifier
            ),
            Self::Check(None) => f.write_str("SCSI command failed"),
            Self::Short { expected, done } => {
                write!(f, "short USB transfer: {done} bytes of {expected}")
            }
        }
    }
}

impl std::error::Error for MscError {}

impl From<BulkError> for MscError {
    fn from(error: BulkError) -> Self {
        Self::Bulk(error)
    }
}

impl From<MscError> for BlockError {
    fn from(error: MscError) -> Self {
        let kind = match &error {
            MscError::Bulk(BulkError::Disconnected) => io::ErrorKind::NotConnected,
            _ => io::ErrorKind::Other,
        };
        Self::Io(io::Error::new(kind, error))
    }
}
