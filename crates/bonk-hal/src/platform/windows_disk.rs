//! Windows : ouverture brute d'un disque entier, verrouillage de ses volumes.
//!
//! Windows refuse d'écrire sur les secteurs d'un volume monté : chaque volume du disque est verrouillé
//! puis démonté, et sa poignée gardée tant que l'écriture dure. Ces gestes exigent d'être administrateur ;
//! sans cela, tout échoue proprement avant d'avoir touché au disque.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::Mutex;

use bonk_core::{BlockDevice, BlockError, check_access, check_geometry};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_UPDATE_PROPERTIES,
};

use super::windows::{ioctl_raw, read_volumes};
use crate::image::Access;
use crate::target::OpenTargetError;
use crate::windows_facts::disk_number;

/// Alignement mémoire exigé par `FILE_FLAG_NO_BUFFERING` : le secteur, 4 Kio au plus sur ces supports.
const MEMORY_ALIGN: usize = 4096;

/// Volumes verrouillés, gardés jusqu'à la relecture de la table ou la fin du processus.
static LOCKED: Mutex<Vec<File>> = Mutex::new(Vec::new());

fn invalid(message: &str) -> BlockError {
    BlockError::Io(io::Error::new(
        io::ErrorKind::InvalidInput,
        message.to_owned(),
    ))
}

/// Refuse tout chemin qui n'est pas un disque entier.
fn check_disk_path(path: &Path) -> Result<(), BlockError> {
    match path.to_str().and_then(disk_number) {
        Some(_) => Ok(()),
        None => Err(invalid("the path is not a whole disk")),
    }
}

fn open_file(path: &Path, access: Access) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(access == Access::ReadWrite)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
        .open(path)
}

/// Le droit d'ouvrir, sans garder le disque ouvert. Pas d'autorisation à demander ici : l'invite du système
/// n'existe pas encore sous Windows, la commande doit être lancée en administrateur.
pub(super) fn prepare(path: &Path, access: Access) -> Result<(), BlockError> {
    check_disk_path(path)?;
    match open_file(path, access) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            Err(BlockError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{}: run the command as administrator", path.display()),
            )))
        }
        Err(error) => Err(BlockError::Io(error)),
    }
}

/// Disque entier ouvert sans tampon du système : la relecture lit bien le support.
pub(super) struct WindowsDisk {
    file: File,
    size: u64,
    block_size: u32,
    read_only: bool,
}

pub(super) fn open(path: &Path, access: Access) -> Result<WindowsDisk, BlockError> {
    check_disk_path(path)?;
    let file = open_file(path, access)?;
    let geometry = ioctl_raw(
        file.as_raw_handle(),
        IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
        size_of::<DISK_GEOMETRY_EX>() + 512,
    )?;
    // SAFETY : l'appel a réussi, le système a écrit la géométrie dans ce tampon aligné.
    let (size, block_size) = unsafe {
        let info = &*geometry.as_ptr().cast::<DISK_GEOMETRY_EX>();
        (
            u64::try_from(info.DiskSize).unwrap_or(0),
            info.Geometry.BytesPerSector,
        )
    };
    check_geometry(size, block_size)?;
    Ok(WindowsDisk {
        file,
        size,
        block_size,
        read_only: access == Access::ReadOnly,
    })
}

/// Tampon aligné en mémoire, pris dans un `Vec` plus grand : aucun `unsafe` pour une exigence de l'API.
fn aligned(storage: &mut Vec<u8>, len: usize) -> &mut [u8] {
    storage.resize(len + MEMORY_ALIGN, 0);
    let start = storage.as_ptr().align_offset(MEMORY_ALIGN);
    &mut storage[start..start + len]
}

impl BlockDevice for WindowsDisk {
    fn size(&self) -> u64 {
        self.size
    }

    fn block_size(&self) -> u32 {
        self.block_size
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        check_access(self.size, self.block_size, offset, buf.len())?;
        let mut storage = Vec::new();
        let bounce = aligned(&mut storage, buf.len());
        let mut done = 0;
        while done < bounce.len() {
            let read = self
                .file
                .seek_read(&mut bounce[done..], offset + done as u64)?;
            if read == 0 {
                return Err(BlockError::Io(io::ErrorKind::UnexpectedEof.into()));
            }
            done += read;
        }
        buf.copy_from_slice(bounce);
        Ok(())
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        if self.read_only {
            return Err(BlockError::ReadOnly);
        }
        check_access(self.size, self.block_size, offset, data.len())?;
        let mut storage = Vec::new();
        let bounce = aligned(&mut storage, data.len());
        bounce.copy_from_slice(data);
        let mut done = 0;
        while done < bounce.len() {
            let written = self
                .file
                .seek_write(&bounce[done..], offset + done as u64)?;
            if written == 0 {
                return Err(BlockError::Io(io::ErrorKind::WriteZero.into()));
            }
            done += written;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), BlockError> {
        if !self.read_only {
            self.file.sync_all()?;
        }
        Ok(())
    }
}

/// Verrouille et démonte chaque volume du disque ; rend les points démontés.
pub(super) fn unmount(path: &str) -> Result<Vec<String>, OpenTargetError> {
    let number = disk_number(path)
        .ok_or_else(|| OpenTargetError::Unmount(format!("{path}: not a whole disk")))?;
    let volumes = read_volumes().map_err(|e| OpenTargetError::Unmount(e.to_string()))?;
    let mut locked = LOCKED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut done = Vec::new();
    for volume in volumes
        .iter()
        .filter(|v| v.extents.iter().any(|e| e.disk == number))
    {
        let failed = |e: &dyn std::fmt::Display| {
            OpenTargetError::Unmount(format!("could not unmount {}: {e}", volume.name))
        };
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(volume.name.trim_end_matches('\\'))
            .map_err(|e| failed(&e))?;
        for code in [FSCTL_LOCK_VOLUME, FSCTL_DISMOUNT_VOLUME] {
            ioctl_raw(file.as_raw_handle(), code, 0).map_err(|e| failed(&e))?;
        }
        locked.push(file);
        done.extend(volume.mount_points.iter().cloned());
    }
    Ok(done)
}

/// Libère les volumes et demande au système de relire la table.
pub(super) fn rescan(path: &Path) -> Result<(), BlockError> {
    check_disk_path(path)?;
    LOCKED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    let file = open_file(path, Access::ReadWrite)?;
    ioctl_raw(file.as_raw_handle(), IOCTL_DISK_UPDATE_PROPERTIES, 0)?;
    Ok(())
}
