//! macOS : ouverture brute d'un disque entier, démontage, autorisation par `authopen`.
//!
//! L'accès passe par `/dev/rdiskN`, sans le tampon du système. Sans droits, `authopen` demande le mot de
//! passe administrateur et remet le descripteur par une socket : Bonk reste sans privilège.

use std::fs::{File, OpenOptions};
use std::io::{self, IoSliceMut};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use bonk_core::{BlockDevice, BlockError, check_access, check_geometry};

use super::Elevation;
use crate::disk::Disk;
use crate::image::Access;
use crate::macos_facts::raw_disk_path;
use crate::target::OpenTargetError;

/// `_IOR('d', 24, u32)` et `_IOR('d', 25, u64)` de `sys/disk.h`.
const DKIOCGETBLOCKSIZE: libc::c_ulong = 0x4004_6418;
const DKIOCGETBLOCKCOUNT: libc::c_ulong = 0x4008_6419;
/// `_IO('d', 22)` de `sys/disk.h`.
const DKIOCSYNCHRONIZECACHE: libc::c_ulong = 0x2000_6416;

/// Descripteur obtenu d'`authopen` avant le démontage, repris à l'ouverture : une seule invite.
static AUTHORIZED: Mutex<Option<(PathBuf, Access, OwnedFd)>> = Mutex::new(None);

fn raw_path(path: &Path) -> Result<String, BlockError> {
    path.to_str().and_then(raw_disk_path).ok_or_else(|| {
        BlockError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the path is not a whole disk",
        ))
    })
}

fn has_access(raw: &str, access: Access) -> bool {
    let Ok(name) = std::ffi::CString::new(raw) else {
        return false;
    };
    let mode = match access {
        Access::ReadOnly => libc::R_OK,
        Access::ReadWrite => libc::R_OK | libc::W_OK,
    };
    // SAFETY : `name` est une chaîne C valide pendant tout l'appel.
    unsafe { libc::access(name.as_ptr(), mode) == 0 }
}

/// Reçoit le descripteur envoyé par `authopen` en donnée de contrôle `SCM_RIGHTS`.
fn receive_fd(socket: &UnixStream) -> io::Result<OwnedFd> {
    let mut byte = [0u8; 1];
    let mut slices = [IoSliceMut::new(&mut byte)];
    // Assez pour un en-tête et un descripteur, aligné comme l'exige `cmsghdr`.
    let mut control = [0u64; 8];
    // SAFETY : structure simple remplie de zéros, puis champs valides pendant tout l'appel.
    let received = unsafe {
        let mut message: libc::msghdr = std::mem::zeroed();
        message.msg_iov = slices.as_mut_ptr().cast();
        message.msg_iovlen = 1;
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = (size_of_val(&control)) as _;
        if libc::recvmsg(socket.as_raw_fd(), &raw mut message, 0) < 0 {
            return Err(io::Error::last_os_error());
        }
        let header = libc::CMSG_FIRSTHDR(&raw const message);
        if header.is_null()
            || (*header).cmsg_level != libc::SOL_SOCKET
            || (*header).cmsg_type != libc::SCM_RIGHTS
        {
            None
        } else {
            Some(
                libc::CMSG_DATA(header)
                    .cast::<libc::c_int>()
                    .read_unaligned(),
            )
        }
    };
    let fd = received.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "authorization refused or cancelled",
        )
    })?;
    // SAFETY : le noyau vient de créer ce descripteur pour ce processus, qui en est seul propriétaire.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn authorize(
    raw: &str,
    access: Access,
    cancelled: &dyn Fn() -> bool,
) -> Result<OwnedFd, BlockError> {
    let (ours, theirs) = UnixStream::pair()?;
    let flags = match access {
        Access::ReadOnly => libc::O_RDONLY,
        Access::ReadWrite => libc::O_RDWR,
    };
    let mut child = Command::new("/usr/libexec/authopen")
        .args(["-stdoutpipe", "-o", &flags.to_string(), raw])
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .spawn()?;
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(BlockError::Io(io::Error::new(
                io::ErrorKind::Interrupted,
                "cancelled during the authorization request",
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // L'enfant a fermé sa copie : sans descripteur reçu, la lecture finit au lieu de bloquer.
    Ok(receive_fd(&ours)?)
}

pub(super) fn prepare(
    path: &Path,
    access: Access,
    elevation: Elevation,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), BlockError> {
    let raw = raw_path(path)?;
    if has_access(&raw, access) {
        return Ok(());
    }
    if elevation == Elevation::Never {
        return Err(BlockError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{raw}: insufficient rights, elevation disabled"),
        )));
    }
    let fd = authorize(&raw, access, cancelled)?;
    *AUTHORIZED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        Some((path.to_path_buf(), access, fd));
    Ok(())
}

/// Disque entier ouvert en accès brut.
pub(super) struct MacDisk {
    file: File,
    size: u64,
    block_size: u32,
    read_only: bool,
}

pub(super) fn open(
    path: &Path,
    access: Access,
    elevation: Elevation,
    cancelled: &dyn Fn() -> bool,
) -> Result<MacDisk, BlockError> {
    let raw = raw_path(path)?;
    let granted = AUTHORIZED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take()
        .filter(|(p, a, _)| p == path && *a == access)
        .map(|(_, _, fd)| File::from(fd));
    let file = match granted {
        Some(file) => file,
        None if has_access(&raw, access) => OpenOptions::new()
            .read(true)
            .write(access == Access::ReadWrite)
            .open(&raw)?,
        None => {
            prepare(path, access, elevation, cancelled)?;
            let (_, _, fd) = AUTHORIZED
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
            File::from(fd)
        }
    };
    let (mut block_size, mut blocks) = (0u32, 0u64);
    // SAFETY : descripteur valide ; chaque requête écrit dans la variable de la taille qu'elle annonce.
    let read = unsafe {
        libc::ioctl(file.as_raw_fd(), DKIOCGETBLOCKSIZE, &raw mut block_size) == 0
            && libc::ioctl(file.as_raw_fd(), DKIOCGETBLOCKCOUNT, &raw mut blocks) == 0
    };
    if !read {
        return Err(BlockError::Io(io::Error::last_os_error()));
    }
    let size = blocks * u64::from(block_size);
    check_geometry(size, block_size)?;
    Ok(MacDisk {
        file,
        size,
        block_size,
        read_only: access == Access::ReadOnly,
    })
}

impl BlockDevice for MacDisk {
    fn size(&self) -> u64 {
        self.size
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, buf.len())?;
        self.file.read_exact_at(buf, offset)?;
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }
        check_access(self.size, self.block_size, offset, data.len())?;
        self.file.write_all_at(data, offset)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        if self.read_only {
            return Ok(());
        }
        // `sync_all` passe par `F_FULLFSYNC`, que le périphérique brut refuse. Un support sans cache (disque
        // RAM) n'a rien à vider et le dit par `ENOTTY`.
        // SAFETY : descripteur valide ; la requête n'a pas d'argument.
        if unsafe { libc::ioctl(self.file.as_raw_fd(), DKIOCSYNCHRONIZECACHE) } != 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ENOTTY) {
                return Err(BlockError::Io(error));
            }
        }
        Ok(())
    }
}

/// Pose exFAT par `diskutil eraseVolume`, qui exige un nom de volume.
pub(super) fn format_exfat(partition: &Path, label: Option<&str>) -> Result<(), BlockError> {
    let output = Command::new("/usr/sbin/diskutil")
        .arg("eraseVolume")
        .arg("ExFAT")
        .arg(label.unwrap_or("BONK"))
        .arg(partition)
        .stdin(Stdio::null())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(BlockError::Io(io::Error::other(format!(
        "diskutil eraseVolume: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))))
}

/// `diskutil unmountDisk` démonte tous les volumes, ceux d'un conteneur APFS porté par ce disque compris.
pub(super) fn unmount(disk: &Disk) -> Result<Vec<String>, OpenTargetError> {
    let output = Command::new("/usr/sbin/diskutil")
        .args(["unmountDisk", &disk.path])
        .output()
        .map_err(|e| OpenTargetError::Unmount(format!("could not unmount {}: {e}", disk.path)))?;
    if !output.status.success() {
        return Err(OpenTargetError::Unmount(format!(
            "could not unmount {}: {}",
            disk.path,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(disk
        .partitions
        .iter()
        .flat_map(|p| p.mount_points.iter().cloned())
        .collect())
}
