use std::fs::File;
use std::io;
use std::os::fd::{FromRawFd, RawFd};
use std::path::Path;
use std::sync::OnceLock;

use bonk_core::format::Filesystem;
use bonk_core::msc::UsbDisk;
use bonk_core::{BlockDevice, BlockError};

use super::{Capability, Elevation, Missing, Platform, writing_capabilities};
use crate::disk::{Bus, Disk, DiskKind};
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::target::OpenTargetError;
use crate::usbfs::UsbfsTransport;

const FD_VARIABLE: &str = "TERMUX_USB_FD";

/// The one drive `termux-usb -E -e` handed over: Android grants it to Termux:API, which passes an open usbfs
/// node in `TERMUX_USB_FD`. Without root there is no other way to a drive.
pub(crate) struct Termux {
    fd: RawFd,
    /// `/dev/bus/usb/BBB/DDD`, the path the drive is named by.
    node: String,
}

fn parse_fd(text: &str) -> Option<RawFd> {
    text.trim().parse().ok().filter(|fd: &RawFd| *fd >= 0)
}

/// Read once: the descriptor lives as long as the process.
pub(crate) fn from_env() -> Option<&'static Termux> {
    static TERMUX: OnceLock<Option<Termux>> = OnceLock::new();
    TERMUX
        .get_or_init(|| {
            let fd = parse_fd(&std::env::var(FD_VARIABLE).ok()?)?;
            let node = std::fs::read_link(format!("/proc/self/fd/{fd}"))
                .ok()
                .map_or_else(|| format!("fd-{fd}"), |path| path.display().to_string());
            Some(Termux { fd, node })
        })
        .as_ref()
}

/// The same shape as the Android app's: serial and plug point, so a cloned key is told apart.
fn disk_id(node: &str, serial: Option<&str>) -> String {
    let port = node
        .strip_prefix("/dev/bus/usb/")
        .unwrap_or(node)
        .replace('/', "-");
    match serial {
        Some(serial) => format!("usb-{serial}-{port}"),
        None => format!("usb-{port}"),
    }
}

impl Termux {
    /// On a copy of the descriptor: each opening claims and releases the drive on its own.
    fn transport(&self) -> io::Result<UsbfsTransport> {
        // SAFETY: `dup` only reads the given descriptor; a failure is checked right after.
        let copy = unsafe { libc::dup(self.fd) };
        if copy < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `copy` was just created by `dup` and has no other owner.
        UsbfsTransport::from_device(unsafe { File::from_raw_fd(copy) })
    }

    fn not_this_drive(&self, path: &Path) -> BlockError {
        BlockError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "{}: only {} was handed over by termux-usb",
                path.display(),
                self.node
            ),
        ))
    }
}

impl Platform for Termux {
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError> {
        let io_error = |source: io::Error| EnumerateError::Io {
            what: self.node.clone(),
            source,
        };
        let mut transport = self.transport().map_err(io_error)?;
        let serial = transport.serial();
        let disk =
            UsbDisk::open(transport).map_err(|e| io_error(io::Error::other(e.to_string())))?;
        let model = disk
            .inquiry()
            .map(|found| {
                format!("{} {}", found.vendor, found.product)
                    .trim()
                    .to_owned()
            })
            .unwrap_or_default();
        Ok(vec![Disk {
            id: disk_id(&self.node, serial.as_deref()),
            path: self.node.clone(),
            model,
            serial,
            size_bytes: disk.size(),
            bus: Bus::Usb,
            kind: DiskKind::Physical,
            removable_flag: true,
            logical_block_size: Some(disk.block_size()),
            location: None,
            fs_type: None,
            holders: Vec::new(),
            partitions: Vec::new(),
        }])
    }

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
        (self.enumerate(), Vec::new())
    }

    fn enumerate_from_capture(&self, _reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
        Err(EnumerateError::Unsupported { os: "termux" })
    }

    /// Claiming the interface detached the system's driver: nothing stays mounted.
    fn unmount(&self, _disk: &Disk, _elevation: Elevation) -> Result<Vec<String>, OpenTargetError> {
        Ok(Vec::new())
    }

    fn prepare_access(
        &self,
        _path: &Path,
        _access: Access,
        _elevation: Elevation,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<(), BlockError> {
        Ok(())
    }

    fn open_disk(
        &self,
        path: &Path,
        _access: Access,
        _elevation: Elevation,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn BlockDevice>, BlockError> {
        if path != Path::new(&self.node) {
            return Err(self.not_this_drive(path));
        }
        let transport = self.transport().map_err(BlockError::Io)?;
        Ok(Box::new(UsbDisk::open(transport)?))
    }

    fn capabilities(&self) -> Vec<Capability> {
        let granted = Capability::no(
            "elevate",
            Missing::OsUnsupported,
            "termux-usb grants access to the drive, nothing else is needed",
        );
        writing_capabilities(granted, |_, name| {
            Capability::no(
                name,
                Missing::OsUnsupported,
                "no system formatting tool reaches the drive in Termux",
            )
        })
    }

    fn format_partition(
        &self,
        partition: &Path,
        _fs: Filesystem,
        _label: Option<&str>,
        _elevation: Elevation,
    ) -> Result<(), BlockError> {
        Err(BlockError::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{}: no system formatting tool reaches the drive in Termux",
                partition.display()
            ),
        )))
    }

    /// No kernel table to reread: the drive is driven over USB, not through a block device.
    fn rescan(&self, _disk: &Path, _elevation: Elevation) -> Result<(), BlockError> {
        Ok(())
    }

    fn is_privileged(&self) -> bool {
        false
    }

    fn kernel_release(&self) -> Option<String> {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|text| text.trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descripteur_lu_dans_la_variable() {
        assert_eq!(parse_fd("7"), Some(7));
        assert_eq!(parse_fd(" 12\n"), Some(12));
        assert_eq!(parse_fd("-1"), None);
        assert_eq!(parse_fd("sept"), None);
        assert_eq!(parse_fd(""), None);
    }

    #[test]
    fn identifiant_comme_celui_de_l_application() {
        assert_eq!(
            disk_id("/dev/bus/usb/001/004", Some("AA00112233")),
            "usb-AA00112233-001-004"
        );
        assert_eq!(disk_id("/dev/bus/usb/002/009", None), "usb-002-009");
        assert_eq!(disk_id("fd-5", None), "usb-fd-5");
    }
}
