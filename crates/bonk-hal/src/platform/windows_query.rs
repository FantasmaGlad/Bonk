//! Windows : collecte des faits sur les disques et les volumes, en lecture seule.
//!
//! Les poignées s'ouvrent sans droit d'accès : les requêtes de propriétés n'en demandent aucun, et la
//! liste des disques n'exige donc pas d'être administrateur. L'interprétation vit dans
//! [`crate::windows_facts`], testée sur tous les systèmes.

use std::collections::HashMap;
use std::ffi::{OsString, c_void};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
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
    DISK_EXTENT, DISK_GEOMETRY_EX, DRIVE_LAYOUT_INFORMATION_EX, GUID_DEVINTERFACE_DISK,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
    IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PARTITION_INFORMATION_EX,
    PARTITION_STYLE_GPT, PARTITION_STYLE_MBR, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageDeviceProperty, VOLUME_DISK_EXTENTS,
};
use windows_sys::core::GUID;

use crate::enumerate::EnumerateError;
use crate::windows_facts::{RawDisk, RawExtent, RawPartition, RawVolume, WindowsFacts};

/// Au-delà, Windows ne numérote pas de disques en pratique ; les numéros peuvent avoir des trous.
const MAX_DISKS: u32 = 128;

/// Poignée refermée à la sortie de portée.
pub(super) struct Handle(pub(super) HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY : la poignée vient de `CreateFileW`, valide, et n'est fermée qu'ici.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(super) fn wide(text: &str) -> Vec<u16> {
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

/// Requête sans entrée sur une poignée déjà ouverte.
pub(super) fn ioctl_raw(handle: HANDLE, code: u32, capacity: usize) -> io::Result<Vec<u64>> {
    ioctl_on(handle, code, &[], capacity)
}

fn ioctl(handle: &Handle, code: u32, input: &[u8], capacity: usize) -> io::Result<Vec<u64>> {
    ioctl_on(handle.0, code, input, capacity)
}

/// Tampon aligné pour les structures de Windows, qui contiennent des entiers de 64 bits.
fn ioctl_on(handle: HANDLE, code: u32, input: &[u8], capacity: usize) -> io::Result<Vec<u64>> {
    let mut size = capacity;
    loop {
        let mut output = vec![0u64; size.div_ceil(8)];
        let mut returned = 0u32;
        // SAFETY : entrée et sortie valides pour les tailles annoncées ; la poignée vit le temps de l'appel.
        let done = unsafe {
            DeviceIoControl(
                handle,
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

/// Partitions réelles du disque.
fn layout(handle: &Handle) -> io::Result<Vec<RawPartition>> {
    let buffer = ioctl(handle, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], 16 * 1024)?;
    let info = buffer.as_ptr().cast::<DRIVE_LAYOUT_INFORMATION_EX>();
    // SAFETY : l'appel a réussi, le système a donc écrit l'en-tête dans ce tampon aligné.
    let header = unsafe { &*info };
    let count = usize::try_from(header.PartitionCount).unwrap_or(0);
    let room = (buffer.len() * 8 - size_of::<DRIVE_LAYOUT_INFORMATION_EX>())
        / size_of::<PARTITION_INFORMATION_EX>()
        + 1;
    // SAFETY : `count` est borné par ce que le tampon peut contenir ; le système a écrit ces entrées.
    let entries =
        unsafe { std::slice::from_raw_parts(header.PartitionEntry.as_ptr(), count.min(room)) };
    Ok(entries
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
        .collect())
}

/// Lit un disque ; `None` s'il n'existe pas ou s'il n'a pas de support (lecteur de carte vide).
fn read_disk(number: u32, instance: Option<String>) -> io::Result<Option<RawDisk>> {
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
    let partitions = layout(&handle)?;
    Ok(Some(RawDisk {
        number,
        bus_type: descriptor.BusType,
        removable: descriptor.RemovableMedia,
        vendor: text_at(raw, descriptor.VendorIdOffset),
        product: text_at(raw, descriptor.ProductIdOffset),
        serial: text_at(raw, descriptor.SerialNumberOffset),
        size_bytes: u64::try_from(size).unwrap_or(0),
        instance,
        partitions,
    }))
}

/// Chemin d'instance de chaque disque, par numéro. Descriptif : sans lui, l'identifiant retombe sur le numéro.
fn disk_instances() -> HashMap<u32, String> {
    let mut found = HashMap::new();
    let class = GUID_DEVINTERFACE_DISK;
    // SAFETY : GUID valide ; les autres arguments sont facultatifs.
    let set = unsafe {
        SetupDiGetClassDevsW(
            &raw const class,
            null(),
            null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if set as isize == -1 {
        return found;
    }
    for index in 0.. {
        // SAFETY : structure simple ; sa taille est annoncée comme l'exige l'API.
        let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        interface.cbSize = u32::try_from(size_of::<SP_DEVICE_INTERFACE_DATA>()).unwrap_or(0);
        // SAFETY : ensemble valide, sortie valide.
        let more = unsafe {
            SetupDiEnumDeviceInterfaces(set, null(), &raw const class, index, &raw mut interface)
        };
        if more == 0 {
            break;
        }
        if let Some(path) = interface_path(set, &interface)
            && let Ok(handle) = open_for_query(&path)
            && let Ok(number) = ioctl(
                &handle,
                IOCTL_STORAGE_GET_DEVICE_NUMBER,
                &[],
                size_of::<STORAGE_DEVICE_NUMBER>(),
            )
        {
            // SAFETY : l'appel a réussi, le système a écrit le numéro dans ce tampon aligné.
            let number = unsafe { (*number.as_ptr().cast::<STORAGE_DEVICE_NUMBER>()).DeviceNumber };
            found.insert(number, path);
        }
    }
    // SAFETY : ensemble valide, fermé une seule fois.
    unsafe {
        SetupDiDestroyDeviceInfoList(set);
    }
    found
}

fn interface_path(set: HDEVINFO, interface: &SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    // Tampon aligné : l'en-tête de la structure précède le chemin, en UTF-16.
    let mut buffer = vec![0u64; 512];
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    let capacity = u32::try_from(buffer.len() * 8).ok()?;
    // SAFETY : tampon assez grand et aligné ; `cbSize` est celui de l'en-tête, comme l'exige l'API.
    let path = unsafe {
        (*detail).cbSize = u32::try_from(size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>()).ok()?;
        if SetupDiGetDeviceInterfaceDetailW(
            set,
            interface,
            detail,
            capacity,
            null_mut(),
            null_mut(),
        ) == 0
        {
            return None;
        }
        let start = (&raw const (*detail).DevicePath).cast::<u16>();
        let room =
            (buffer.len() * 8 - offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath)) / 2;
        from_wide(std::slice::from_raw_parts(start, room))
    };
    (!path.is_empty()).then_some(path)
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

pub(super) fn read_volumes() -> io::Result<Vec<RawVolume>> {
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

pub(super) fn collect() -> Result<WindowsFacts, EnumerateError> {
    let failed = |what: &str| {
        let what = what.to_owned();
        move |source| EnumerateError::Io { what, source }
    };
    let mut instances = disk_instances();
    let mut disks = Vec::new();
    for number in 0..MAX_DISKS {
        let instance = instances.remove(&number);
        if let Some(disk) =
            read_disk(number, instance).map_err(failed(&format!("PhysicalDrive{number}")))?
        {
            disks.push(disk);
        }
    }
    Ok(WindowsFacts {
        system_drive: std::env::var("SystemDrive").ok(),
        disks,
        volumes: read_volumes().map_err(failed("volumes"))?,
    })
}
