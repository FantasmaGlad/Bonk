//! Windows : ouverture brute d'un disque entier, verrouillage de ses volumes.
//!
//! Windows refuse d'écrire sur les secteurs d'un volume monté : chaque volume du disque est verrouillé
//! puis démonté, et sa poignée gardée tant que l'écriture dure. Ces gestes exigent d'être administrateur :
//! ce processus l'est, ou c'est le helper élevé qui les fait (`windows_helper`).

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Mutex;

use bonk_core::format::Filesystem;
use bonk_core::{BlockDevice, BlockError, check_access, check_geometry};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_UPDATE_PROPERTIES,
};

use super::windows_query::{ioctl_raw, read_volumes};
use crate::image::Access;
use crate::target::OpenTargetError;
use crate::windows_facts::{disk_number, partition_numbers};

/// Alignement mémoire exigé par `FILE_FLAG_NO_BUFFERING` : le secteur, 4 Kio au plus sur ces supports.
const MEMORY_ALIGN: usize = 4096;

/// Pose exFAT ou NTFS par `Format-Volume`. L'étiquette passe par l'environnement : rien de ce qu'elle
/// contient n'est interprété par PowerShell.
pub(super) fn format_partition(
    partition: &Path,
    fs: Filesystem,
    label: Option<&str>,
) -> Result<(), BlockError> {
    let (disk, number) = partition
        .to_str()
        .and_then(partition_numbers)
        .ok_or_else(|| invalid("the path is not a partition"))?;
    let name = match fs {
        Filesystem::Exfat => "exFAT",
        Filesystem::Ntfs => "NTFS",
        Filesystem::Fat32 | Filesystem::Ext4 => {
            return Err(BlockError::Io(io::ErrorKind::Unsupported.into()));
        }
    };
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $p = Get-Partition -DiskNumber {disk} -PartitionNumber {number}; \
         $a = @{{Partition=$p; FileSystem='{name}'; Force=$true; Confirm=$false}}; \
         if ($env:BONK_FORMAT_LABEL) {{ $a.NewFileSystemLabel = $env:BONK_FORMAT_LABEL }}; \
         Format-Volume @a | Out-Null"
    );
    let mut command = Command::new("powershell.exe");
    crate::console::hide_child_console(&mut command);
    let output = command
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .env("BONK_FORMAT_LABEL", label.unwrap_or_default())
        .stdin(Stdio::null())
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(BlockError::Io(io::Error::other(format!(
        "Format-Volume: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))))
}

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

pub(super) fn open_file(path: &Path, access: Access) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(access == Access::ReadWrite)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH)
        .open(path)
}

/// Le droit d'ouvrir, sans garder le disque ouvert, avec les droits de ce processus.
pub(super) fn prepare(path: &Path, access: Access) -> Result<(), BlockError> {
    check_disk_path(path)?;
    match open_file(path, access) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            Err(BlockError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{}: insufficient rights, elevation disabled",
                    path.display()
                ),
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
    from_file(open_file(path, access)?, access)
}

/// Reprend un disque déjà ouvert, ici ou par le helper. Sa taille se lit sur la poignée : c'est bien celui-là.
pub(super) fn from_file(file: File, access: Access) -> Result<WindowsDisk, BlockError> {
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
