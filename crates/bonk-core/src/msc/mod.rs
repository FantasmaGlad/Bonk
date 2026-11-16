mod disk;
mod session;
#[cfg(test)]
mod tests;
mod transport;

pub use disk::UsbDisk;
pub use session::{Inquiry, MassStorage};
pub use transport::{BulkError, BulkTransport, Direction, MscError, Sense};

const CBW_SIGNATURE: u32 = 0x4342_5355;
const CSW_SIGNATURE: u32 = 0x5342_5355;
const CBW_LEN: usize = 31;
const CSW_LEN: usize = 13;
/// 1 MiB per command: keys accept it and cancel stays quick.
const MAX_TRANSFER: usize = 1 << 20;
const READY_ATTEMPTS: u32 = 5;
const SENSE_ILLEGAL_REQUEST: u8 = 5;
const ASC_INVALID_OPCODE: u8 = 0x20;
