//! Windows : collecte des faits sur les disques et les volumes, en lecture seule.
//!
//! Les poignées s'ouvrent sans droit d'accès : les requêtes de propriétés n'en demandent aucun, et la
//! liste des disques n'exige donc pas d'être administrateur. L'interprétation vit dans
//! [`crate::windows_facts`], testée sur tous les systèmes.

use std::ffi::{OsString, c_void};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::time::Instant;

use bonk_core::format::Filesystem;
use bonk_core::{BlockDevice, BlockError};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_FILE_NOT_FOUND, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FUNCTION,
    ERROR_MORE_DATA, ERROR_NO_MEDIA_IN_DRIVE, ERROR_NO_MORE_FILES, ERROR_NOT_READY,
    ERROR_PATH_NOT_FOUND, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW, FindNextVolumeW,
    FindVolumeClose, GetVolumeInformationW, GetVolumePathNamesForVolumeNameW,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    DISK_EXTENT, DISK_GEOMETRY_EX, DRIVE_LAYOUT_INFORMATION_EX, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
    IOCTL_DISK_GET_DRIVE_LAYOUT_EX, IOCTL_STORAGE_QUERY_PROPERTY, PARTITION_INFORMATION_EX,
    PARTITION_STYLE_GPT, PARTITION_STYLE_MBR, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY, StorageDeviceProperty, VOLUME_DISK_EXTENTS,
};
use windows_sys::core::GUID;

use super::{Capability, Elevation, Platform, Unsupported};
use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::target::OpenTargetError;
use crate::windows_facts::{
    RawDisk, RawExtent, RawPartition, RawVolume, SOURCE, WindowsFacts, normalize,
};

/// Au-delà, Windows ne numérote pas de disques en pratique ; les numéros peuvent avoir des trous.
const MAX_DISKS: u32 = 128;

/// Les fonctions d'écriture restent celles d'un système sans accès brut jusqu'à leur arrivée.
#[derive(Debug, Clone, Copy, Default)]
pub struct Windows;

/// Poignée refermée à la sortie de portée.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY : la poignée vient de `CreateFileW`, valide, et n'est fermée qu'ici.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end])
        .to_string_lossy()
        .into_owned()
}

fn open_for_query(path: &str) -> io::Result<Handle> {
    let name = wide(path);
    // SAFETY : `name` est terminé par un zéro et vit le temps de l'appel ; accès 0 : aucune lecture ni
    // écriture n'est possible par cette poignée.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(handle))
    }
}

fn has_code(error: &io::Error, codes: &[u32]) -> bool {
    error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        .is_some_and(|code| codes.contains(&code))
}

/// Tampon aligné pour les structures de Windows, qui contiennent des entiers de 64 bits.
fn ioctl(handle: &Handle, code: u32, input: &[u8], capacity: usize) -> io::Result<Vec<u64>> {
    let mut size = capacity;
    loop {
        let mut output = vec![0u64; size.div_ceil(8)];
        let mut returned = 0u32;
        // SAFETY : entrée et sortie valides pour les tailles annoncées ; la poignée vit le temps de l'appel.
        let done = unsafe {
            DeviceIoControl(
                handle.0,
                code,
                input.as_ptr().cast::<c_void>(),
                u32::try_from(input.len()).unwrap_or(u32::MAX),
                output.as_mut_ptr().cast::<c_void>(),
                u32::try_from(output.len() * 8).unwrap_or(u32::MAX),
                &raw mut returned,
                null_mut(),
            )
        };
        if done != 0 {
            return Ok(output);
        }
        let error = io::Error::last_os_error();
        if has_code(&error, &[ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA]) && size < 1 << 20 {
            size *= 4;
            continue;
        }
        return Err(error);
    }
}

fn text_at(raw: &[u8], offset: u32) -> Option<String> {
    let offset = usize::try_from(offset).ok().filter(|&o| o != 0)?;
    let tail = raw.get(offset..)?;
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    let text = String::from_utf8_lossy(&tail[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn as_bytes(buffer: &[u64]) -> &[u8] {
    // SAFETY : un tampon de `u64` se lit toujours comme des octets, sur la même longueur.
    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8) }
}

fn guid_text(guid: &GUID) -> String {
    let d4 = guid.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1, guid.data2, guid.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

/// Disposition du disque : son identifiant et ses partitions réelles.
fn layout(handle: &Handle) -> io::Result<(Option<String>, Vec<RawPartition>)> {
    let buffer = ioctl(handle, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], 16 * 1024)?;
    let info = buffer.as_ptr().cast::<DRIVE_LAYOUT_INFORMATION_EX>();
    // SAFETY : l'appel a réussi, le système a donc écrit l'en-tête dans ce tampon aligné.
    let header = unsafe { &*info };
    let style = i32::try_from(header.PartitionStyle).unwrap_or(-1);
    let disk_id = match style {
        // SAFETY : le style indique le membre valide de l'union.
        PARTITION_STYLE_GPT => Some(guid_text(unsafe { &header.Anonymous.Gpt.DiskId })),
        PARTITION_STYLE_MBR => Some(format!("{:08x}", unsafe { header.Anonymous.Mbr.Signature })),
        _ => None,
    };
    let count = usize::try_from(header.PartitionCount).unwrap_or(0);
    let room = (buffer.len() * 8 - size_of::<DRIVE_LAYOUT_INFORMATION_EX>())
        / size_of::<PARTITION_INFORMATION_EX>()
        + 1;
    // SAFETY : `count` est borné par ce que le tampon peut contenir ; le système a écrit ces entrées.
    let entries =
        unsafe { std::slice::from_raw_parts(header.PartitionEntry.as_ptr(), count.min(room)) };
    let partitions = entries
        .iter()
        // Sous MBR, Windows liste aussi des entrées vides, de numéro 0.
        .filter(|entry| entry.PartitionNumber != 0 && entry.PartitionLength > 0)
        .map(|entry| RawPartition {
            number: entry.PartitionNumber,
            offset: u64::try_from(entry.StartingOffset).unwrap_or(0),
            size_bytes: u64::try_from(entry.PartitionLength).unwrap_or(0),
            // SAFETY : le style de l'entrée indique le membre valide de l'union.
            gpt_type: (entry.PartitionStyle == PARTITION_STYLE_GPT)
                .then(|| guid_text(unsafe { &entry.Anonymous.Gpt.PartitionType })),
            mbr_type: if entry.PartitionStyle == PARTITION_STYLE_MBR {
                // SAFETY : le style de l'entrée indique le membre valide de l'union.
                Some(unsafe { entry.Anonymous.Mbr.PartitionType })
            } else {
                None
            },
        })
        .collect();
    Ok((disk_id, partitions))
}

/// Lit un disque ; `None` s'il n'existe pas ou s'il n'a pas de support (lecteur de carte vide).
fn read_disk(number: u32) -> io::Result<Option<RawDisk>> {
    let handle = match open_for_query(&format!(r"\\.\PhysicalDrive{number}")) {
        Ok(handle) => handle,
        Err(error) if has_code(&error, &[ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND]) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // SAFETY : structure simple, lue comme des octets sur sa propre taille.
    let input = unsafe {
        std::slice::from_raw_parts(
            (&raw const query).cast::<u8>(),
            size_of::<STORAGE_PROPERTY_QUERY>(),
        )
    };
    let property = ioctl(&handle, IOCTL_STORAGE_QUERY_PROPERTY, input, 4096)?;
    // SAFETY : l'appel a réussi, le système a écrit le descripteur dans ce tampon aligné.
    let descriptor = unsafe { &*property.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>() };
    let raw = as_bytes(&property);

    let geometry = match ioctl(
        &handle,
        IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
        &[],
        size_of::<DISK_GEOMETRY_EX>() + 512,
    ) {
        Ok(geometry) => geometry,
        Err(error) if has_code(&error, &[ERROR_NOT_READY, ERROR_NO_MEDIA_IN_DRIVE]) => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    // SAFETY : l'appel a réussi, le système a écrit la géométrie dans ce tampon aligné.
    let size = unsafe { (*geometry.as_ptr().cast::<DISK_GEOMETRY_EX>()).DiskSize };
    let (disk_id, partitions) = layout(&handle)?;
    Ok(Some(RawDisk {
        number,
        bus_type: descriptor.BusType,
        removable: descriptor.RemovableMedia,
        vendor: text_at(raw, descriptor.VendorIdOffset),
        product: text_at(raw, descriptor.ProductIdOffset),
        serial: text_at(raw, descriptor.SerialNumberOffset),
        size_bytes: u64::try_from(size).unwrap_or(0),
        disk_id,
        partitions,
    }))
}

fn mount_points(volume: &str) -> io::Result<Vec<String>> {
    let name = wide(volume);
    let mut buffer = vec![0u16; 1024];
    loop {
        let mut needed = 0u32;
        // SAFETY : nom terminé par un zéro, tampon valide pour la longueur annoncée.
        let done = unsafe {
            GetVolumePathNamesForVolumeNameW(
                name.as_ptr(),
                buffer.as_mut_ptr(),
                u32::try_from(buffer.len()).unwrap_or(u32::MAX),
                &raw mut needed,
            )
        };
        if done != 0 {
            break;
        }
        let error = io::Error::last_os_error();
        if has_code(&error, &[ERROR_MORE_DATA]) && needed as usize > buffer.len() {
            buffer.resize(needed as usize, 0);
            continue;
        }
        return Err(error);
    }
    Ok(buffer
        .split(|&c| c == 0)
        .take_while(|part| !part.is_empty())
        .map(from_wide)
        .collect())
}

/// Morceaux du volume sur les disques ; `None` pour un volume sans disque (lecteur optique, vide).
fn extents(volume: &str) -> io::Result<Option<Vec<RawExtent>>> {
    // Sans la barre finale : avec elle, le nom désigne le dossier racine et non le volume.
    let handle = open_for_query(volume.trim_end_matches('\\'))?;
    let buffer = match ioctl(
        &handle,
        IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
        &[],
        size_of::<VOLUME_DISK_EXTENTS>() + 16 * size_of::<DISK_EXTENT>(),
    ) {
        Ok(buffer) => buffer,
        Err(error)
            if has_code(
                &error,
                &[
                    ERROR_INVALID_FUNCTION,
                    ERROR_NOT_READY,
                    ERROR_NO_MEDIA_IN_DRIVE,
                ],
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    // SAFETY : l'appel a réussi, le système a écrit l'en-tête dans ce tampon aligné.
    let header = unsafe { &*buffer.as_ptr().cast::<VOLUME_DISK_EXTENTS>() };
    let count = usize::try_from(header.NumberOfDiskExtents).unwrap_or(0);
    let room = (buffer.len() * 8 - size_of::<VOLUME_DISK_EXTENTS>()) / size_of::<DISK_EXTENT>() + 1;
    // SAFETY : `count` est borné par ce que le tampon peut contenir ; le système a écrit ces entrées.
    let entries = unsafe { std::slice::from_raw_parts(header.Extents.as_ptr(), count.min(room)) };
    Ok(Some(
        entries
            .iter()
            .map(|extent| RawExtent {
                disk: extent.DiskNumber,
                offset: u64::try_from(extent.StartingOffset).unwrap_or(0),
            })
            .collect(),
    ))
}

/// Système de fichiers et étiquette : descriptifs, un volume illisible n'en a simplement pas.
fn volume_information(volume: &str) -> (Option<String>, Option<String>) {
    let name = wide(volume);
    let mut label = [0u16; 261];
    let mut file_system = [0u16; 261];
    // SAFETY : nom terminé par un zéro, tampons valides pour les longueurs annoncées, autres sorties
    // facultatives.
    let done = unsafe {
        GetVolumeInformationW(
            name.as_ptr(),
            label.as_mut_ptr(),
            261,
            null_mut(),
            null_mut(),
            null_mut(),
            file_system.as_mut_ptr(),
            261,
        )
    };
    if done == 0 {
        return (None, None);
    }
    let not_empty = |text: String| (!text.is_empty()).then_some(text);
    (
        not_empty(from_wide(&file_system)),
        not_empty(from_wide(&label)),
    )
}

fn read_volumes() -> io::Result<Vec<RawVolume>> {
    let mut name = [0u16; 64];
    // SAFETY : tampon valide pour la longueur annoncée.
    let find = unsafe { FindFirstVolumeW(name.as_mut_ptr(), 64) };
    if find == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let mut volumes = Vec::new();
    let result = loop {
        let volume = from_wide(&name);
        match (extents(&volume), mount_points(&volume)) {
            (Ok(Some(extents)), Ok(mount_points)) => {
                let (file_system, label) = volume_information(&volume);
                volumes.push(RawVolume {
                    name: volume,
                    mount_points,
                    file_system,
                    label,
                    extents,
                });
            }
            (Ok(None), _) => {}
            (Err(error), _) | (_, Err(error)) => break Err(error),
        }
        // SAFETY : `find` est valide jusqu'à `FindVolumeClose` ; tampon valide pour la longueur annoncée.
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), 64) } == 0 {
            let error = io::Error::last_os_error();
            break if has_code(&error, &[ERROR_NO_MORE_FILES]) {
                Ok(())
            } else {
                Err(error)
            };
        }
    };
    // SAFETY : `find` vient de `FindFirstVolumeW` et n'est fermée qu'ici.
    unsafe {
        FindVolumeClose(find);
    }
    result.map(|()| volumes)
}

fn collect() -> Result<WindowsFacts, EnumerateError> {
    let failed = |what: &str| {
        let what = what.to_owned();
        move |source| EnumerateError::Io { what, source }
    };
    let mut disks = Vec::new();
    for number in 0..MAX_DISKS {
        if let Some(disk) = read_disk(number).map_err(failed(&format!("PhysicalDrive{number}")))? {
            disks.push(disk);
        }
    }
    Ok(WindowsFacts {
        system_drive: std::env::var("SystemDrive").ok(),
        disks,
        volumes: read_volumes().map_err(failed("volumes"))?,
    })
}

impl Platform for Windows {
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError> {
        normalize(&collect()?)
    }

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
        let started = Instant::now();
        let facts = collect();
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        match facts {
            Ok(facts) => {
                let content = serde_json::to_string(&facts).unwrap_or_default();
                let read = SourceRead {
                    path: SOURCE.to_owned(),
                    kind: "file".to_owned(),
                    ok: true,
                    bytes: Some(content.len()),
                    error: None,
                    micros,
                    content: Some(content),
                };
                (normalize(&facts), vec![read])
            }
            Err(error) => {
                let read = SourceRead {
                    path: SOURCE.to_owned(),
                    kind: "file".to_owned(),
                    ok: false,
                    bytes: None,
                    error: Some(error.to_string()),
                    micros,
                    content: None,
                };
                (Err(error), vec![read])
            }
        }
    }

    fn enumerate_from_capture(&self, _reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
        Err(EnumerateError::Malformed {
            what: "capture".to_owned(),
            detail: "no Windows facts in this capture".to_owned(),
        })
    }

    fn unmount(&self, disk: &Disk, elevation: Elevation) -> Result<Vec<String>, OpenTargetError> {
        Unsupported.unmount(disk, elevation)
    }

    fn prepare_access(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), BlockError> {
        Unsupported.prepare_access(path, access, elevation, cancelled)
    }

    fn open_disk(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn BlockDevice>, BlockError> {
        Unsupported.open_disk(path, access, elevation, cancelled)
    }

    fn capabilities(&self) -> Vec<Capability> {
        Unsupported.capabilities()
    }

    fn format_partition(
        &self,
        partition: &Path,
        fs: Filesystem,
        label: Option<&str>,
        elevation: Elevation,
    ) -> Result<(), BlockError> {
        Unsupported.format_partition(partition, fs, label, elevation)
    }

    fn rescan(&self, disk: &Path, elevation: Elevation) -> Result<(), BlockError> {
        Unsupported.rescan(disk, elevation)
    }

    fn is_privileged(&self) -> bool {
        false
    }

    fn kernel_release(&self) -> Option<String> {
        None
    }
}
