use super::*;

pub(super) const fn request(direction: u64, nr: u64, size: usize) -> u64 {
    (direction << 30) | ((size as u64) << 16) | ((b'U' as u64) << 8) | nr
}

pub(super) const READ: u64 = 2;
pub(super) const WRITE_READ: u64 = 3;

#[repr(C)]
pub(super) struct BulkTransfer {
    pub(super) endpoint: u32,
    pub(super) length: u32,
    pub(super) timeout_ms: u32,
    pub(super) data: *mut libc::c_void,
}

#[repr(C)]
pub(super) struct ControlTransfer {
    pub(super) request_type: u8,
    pub(super) request: u8,
    pub(super) value: u16,
    pub(super) index: u16,
    pub(super) length: u16,
    pub(super) timeout_ms: u32,
    pub(super) data: *mut libc::c_void,
}

#[repr(C)]
pub(super) struct InterfaceIoctl {
    pub(super) interface: libc::c_int,
    pub(super) code: libc::c_int,
    pub(super) data: *mut libc::c_void,
}

pub(super) const USBDEVFS_CONTROL: u64 = request(WRITE_READ, 0, size_of::<ControlTransfer>());
pub(super) const USBDEVFS_BULK: u64 = request(WRITE_READ, 2, size_of::<BulkTransfer>());
pub(super) const USBDEVFS_CLAIMINTERFACE: u64 = request(READ, 15, size_of::<u32>());
pub(super) const USBDEVFS_RELEASEINTERFACE: u64 = request(READ, 16, size_of::<u32>());
pub(super) const USBDEVFS_IOCTL: u64 = request(WRITE_READ, 18, size_of::<InterfaceIoctl>());
pub(super) const USBDEVFS_CLEAR_HALT: u64 = request(READ, 21, size_of::<u32>());
pub(super) const USBDEVFS_DISCONNECT: u64 = request(0, 22, 0);
pub(super) const USBDEVFS_CONNECT: u64 = request(0, 23, 0);

/// Out of memory, the kernel refuses before touching the bus: retry smaller, and keep the smaller size.
pub(super) fn with_shrinking_limit(
    limit: &mut usize,
    wanted: usize,
    mut transfer: impl FnMut(usize) -> io::Result<usize>,
) -> io::Result<usize> {
    loop {
        match transfer(wanted.min(*limit)) {
            Err(error) if error.raw_os_error() == Some(libc::ENOMEM) && *limit > TRANSFER_MIN => {
                *limit = (*limit / 2).max(TRANSFER_MIN);
                log::debug!(target: "bonk::hal", limit = *limit; "transfer size reduced");
            }
            result => return result,
        }
    }
}

/// A transfer is capped at `limit`: chain until `len` bytes or a short packet, the only sign the key is done
/// (`BulkTransport::receive` contract). `transfer(done, size)` reads at most `size` bytes after the `done`
/// already read.
pub(super) fn receive_chained(
    len: usize,
    limit: &mut usize,
    mut transfer: impl FnMut(usize, usize) -> io::Result<usize>,
) -> io::Result<usize> {
    let mut done = 0;
    while done < len {
        let asked = (len - done).min(*limit);
        let read = with_shrinking_limit(limit, asked, |size| transfer(done, size))?;
        done += read;
        if read < asked.min(*limit) {
            break;
        }
    }
    Ok(done)
}

pub(super) fn bulk_error(error: io::Error) -> BulkError {
    match error.raw_os_error() {
        Some(libc::EPIPE) => BulkError::Stall,
        Some(libc::ENODEV | libc::ENOENT | libc::ESHUTDOWN) => BulkError::Disconnected,
        _ => BulkError::Other(error),
    }
}
