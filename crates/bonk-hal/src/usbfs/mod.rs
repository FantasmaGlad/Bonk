use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::path::Path;

use bonk_core::msc::{BulkError, BulkTransport, Direction};

mod descriptor;
mod kernel;
#[cfg(test)]
mod tests;

pub use descriptor::StorageInterface;
use descriptor::*;
use kernel::*;

/// No answer within this, the key is lost. Flushing its cache can take long.
const TIMEOUT_MS: u32 = 60_000;

/// The kernel allocates its buffer in one piece: on a phone with fragmented memory 1 MiB ends up refused
/// while memory remains, and the write stopped there.
const TRANSFER_START: usize = 256 << 10;
const TRANSFER_MIN: usize = 16 << 10;

const CLASS_STORAGE: u8 = 0x08;
const SUBCLASS_SCSI: u8 = 0x06;
const PROTOCOL_BULK_ONLY: u8 = 0x50;
const DESCRIPTOR_INTERFACE: u8 = 4;
const DESCRIPTOR_ENDPOINT: u8 = 5;
const ATTRIBUTES_BULK: u8 = 2;
const ENDPOINT_IN: u8 = 0x80;
const REQUEST_MASS_STORAGE_RESET: u8 = 0xFF;
const REQUEST_GET_DESCRIPTOR: u8 = 6;
const REQUEST_TYPE_STANDARD_DEVICE_IN: u8 = 0x80;
const DESCRIPTOR_STRING: u16 = 3;
/// `iSerialNumber` in the device descriptor, always first in what the node gives.
const DEVICE_SERIAL_INDEX: usize = 16;
const REQUEST_TYPE_CLASS_INTERFACE_OUT: u8 = 0x21;

/// Releasing it gives the key back to the kernel driver.
#[derive(Debug)]
pub struct UsbfsTransport {
    device: File,
    /// Device descriptor first, as read from the node; empty when Android handed them separately.
    descriptors: Vec<u8>,
    interface: StorageInterface,
    /// Claimed here, released here. On Android the app holds it.
    claimed_here: bool,
    transfer_limit: usize,
}

impl UsbfsTransport {
    /// Opens `/dev/bus/usb/BBB/DDD`, detaches the kernel driver and claims the storage interface.
    ///
    /// # Errors
    /// Missing rights, missing device, or no bulk-only storage interface.
    pub fn open(path: &Path) -> io::Result<Self> {
        Self::from_device(OpenOptions::new().read(true).write(true).open(path)?)
    }

    /// On a descriptor already open, as Android gives it.
    ///
    /// # Errors
    /// No bulk-only storage interface, or claim refused.
    pub(crate) fn from_device(mut device: File) -> io::Result<Self> {
        let mut descriptors = Vec::new();
        device.read_to_end(&mut descriptors)?;
        let interface = find_storage_interface(&descriptors).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "no bulk-only storage interface")
        })?;
        let mut transport = Self {
            device,
            descriptors,
            interface,
            // Set once claimed: before that, dropping must not release what was never taken.
            claimed_here: false,
            transfer_limit: TRANSFER_START,
        };
        let mut detach = InterfaceIoctl {
            interface: libc::c_int::from(interface.number),
            code: USBDEVFS_DISCONNECT as libc::c_int,
            data: std::ptr::null_mut(),
        };
        // Without a driver attached the system answers ENODATA: the way is already free.
        if let Err(error) = transport.ioctl(USBDEVFS_IOCTL, (&raw mut detach).cast())
            && error.raw_os_error() != Some(libc::ENODATA)
        {
            return Err(error);
        }
        let mut number = u32::from(interface.number);
        if let Err(error) = transport.ioctl(USBDEVFS_CLAIMINTERFACE, (&raw mut number).cast()) {
            // The kernel driver was detached above: give the drive back to it.
            if let Err(reattach) = transport.reattach_driver() {
                log::warn!(target: "bonk::hal", error:% = reattach; "the drive was not handed back to the kernel driver");
            }
            return Err(error);
        }
        transport.claimed_here = true;
        Ok(transport)
    }

    /// On a descriptor whose interface is already claimed: Android does it with `claimInterface(interface,
    /// true)` and gives the descriptors with `getRawDescriptors()`.
    ///
    /// # Errors
    /// No bulk-only storage interface.
    pub fn from_claimed(device: File, descriptors: &[u8]) -> io::Result<Self> {
        let interface = find_storage_interface(descriptors).ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "no bulk-only storage interface")
        })?;
        Ok(Self {
            device,
            descriptors: descriptors.to_vec(),
            interface,
            claimed_here: false,
            transfer_limit: TRANSFER_START,
        })
    }

    fn ioctl(&mut self, code: u64, argument: *mut libc::c_void) -> io::Result<usize> {
        // SAFETY: `argument` points to the struct `code` expects, alive during the call, and the buffers it
        // names have the declared length. The cast follows the libc's `ioctl` type.
        let done = unsafe { libc::ioctl(self.device.as_raw_fd(), code as _, argument) };
        usize::try_from(done).map_err(|_| io::Error::last_os_error())
    }

    fn bulk_once(&mut self, endpoint: u8, data: *mut u8, size: usize) -> io::Result<usize> {
        let mut transfer = BulkTransfer {
            endpoint: u32::from(endpoint),
            // `size` never exceeds `TRANSFER_START`.
            length: size as u32,
            timeout_ms: TIMEOUT_MS,
            data: data.cast(),
        };
        self.ioctl(USBDEVFS_BULK, (&raw mut transfer).cast())
    }

    fn bulk(&mut self, endpoint: u8, data: *mut u8, length: usize) -> Result<usize, BulkError> {
        let mut limit = self.transfer_limit;
        let result = with_shrinking_limit(&mut limit, length, |size| {
            self.bulk_once(endpoint, data, size)
        });
        self.transfer_limit = limit;
        result.map_err(bulk_error)
    }
}

impl BulkTransport for UsbfsTransport {
    fn send(&mut self, data: &[u8]) -> Result<(), BulkError> {
        let endpoint = self.interface.endpoint_out;
        let mut sent = 0;
        while sent < data.len() {
            // The system only reads this buffer when sending.
            let rest = data[sent..].as_ptr().cast_mut();
            match self.bulk(endpoint, rest, data.len() - sent)? {
                0 => {
                    return Err(BulkError::Other(io::Error::other(
                        "the drive accepted nothing",
                    )));
                }
                count => sent += count,
            }
        }
        Ok(())
    }

    fn receive(&mut self, buf: &mut [u8]) -> Result<usize, BulkError> {
        let endpoint = self.interface.endpoint_in;
        let mut limit = self.transfer_limit;
        let len = buf.len();
        let result = receive_chained(len, &mut limit, |done, size| {
            self.bulk_once(endpoint, buf[done..].as_mut_ptr(), size)
        });
        self.transfer_limit = limit;
        result.map_err(bulk_error)
    }

    fn clear_halt(&mut self, direction: Direction) -> Result<(), BulkError> {
        let mut endpoint = u32::from(match direction {
            Direction::In => self.interface.endpoint_in,
            Direction::Out => self.interface.endpoint_out,
        });
        self.ioctl(USBDEVFS_CLEAR_HALT, (&raw mut endpoint).cast())
            .map(|_| ())
            .map_err(bulk_error)
    }

    fn reset(&mut self) -> Result<(), BulkError> {
        let mut control = ControlTransfer {
            request_type: REQUEST_TYPE_CLASS_INTERFACE_OUT,
            request: REQUEST_MASS_STORAGE_RESET,
            value: 0,
            index: u16::from(self.interface.number),
            length: 0,
            timeout_ms: TIMEOUT_MS,
            data: std::ptr::null_mut(),
        };
        self.ioctl(USBDEVFS_CONTROL, (&raw mut control).cast())
            .map_err(bulk_error)?;
        self.clear_halt(Direction::In)?;
        self.clear_halt(Direction::Out)
    }
}

impl UsbfsTransport {
    /// The serial the drive declares, read in the first language it offers; `None` without one.
    pub fn serial(&mut self) -> Option<String> {
        let index = *self
            .descriptors
            .get(DEVICE_SERIAL_INDEX)
            .filter(|&&i| i != 0)?;
        let languages = self.string_descriptor(0, 0)?;
        let language = u16::from_le_bytes([*languages.get(2)?, *languages.get(3)?]);
        string_of(&self.string_descriptor(index, language)?)
    }

    fn string_descriptor(&mut self, index: u8, language: u16) -> Option<Vec<u8>> {
        let mut buffer = [0u8; 255];
        let mut control = ControlTransfer {
            request_type: REQUEST_TYPE_STANDARD_DEVICE_IN,
            request: REQUEST_GET_DESCRIPTOR,
            value: (DESCRIPTOR_STRING << 8) | u16::from(index),
            index: language,
            length: buffer.len() as u16,
            timeout_ms: TIMEOUT_MS,
            data: buffer.as_mut_ptr().cast(),
        };
        let read = self
            .ioctl(USBDEVFS_CONTROL, (&raw mut control).cast())
            .ok()?;
        Some(buffer[..read.min(buffer.len())].to_vec())
    }

    fn reattach_driver(&mut self) -> io::Result<usize> {
        let mut attach = InterfaceIoctl {
            interface: libc::c_int::from(self.interface.number),
            code: USBDEVFS_CONNECT as libc::c_int,
            data: std::ptr::null_mut(),
        };
        self.ioctl(USBDEVFS_IOCTL, (&raw mut attach).cast())
    }
}

impl Drop for UsbfsTransport {
    fn drop(&mut self) {
        if !self.claimed_here {
            return;
        }
        let mut number = u32::from(self.interface.number);
        let released = self.ioctl(USBDEVFS_RELEASEINTERFACE, (&raw mut number).cast());
        let attached = self.reattach_driver();
        if released.is_err() || attached.is_err() {
            log::warn!(target: "bonk::hal", "the drive was not handed back to the kernel driver");
        }
    }
}
