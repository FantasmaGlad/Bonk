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

/// Windows does not number disks beyond this in practice; numbers may have gaps.
const MAX_DISKS: u32 = 128;

pub(super) struct Handle(pub(super) HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle comes from `CreateFileW`, valid, and is closed only here.
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
    // SAFETY: `name` is zero-terminated and lives for the call; access 0: nothing can be read or written
    // through this handle.
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

pub(super) fn ioctl_raw(handle: HANDLE, code: u32, capacity: usize) -> io::Result<Vec<u64>> {
    ioctl_on(handle, code, &[], capacity)
}

fn ioctl(handle: &Handle, code: u32, input: &[u8], capacity: usize) -> io::Result<Vec<u64>> {
    ioctl_on(handle.0, code, input, capacity)
}

fn ioctl_on(handle: HANDLE, code: u32, input: &[u8], capacity: usize) -> io::Result<Vec<u64>> {
    let mut size = capacity;
    loop {
        let mut output = vec![0u64; size.div_ceil(8)];
        let mut returned = 0u32;
        // SAFETY: input and output valid for the declared sizes; the handle lives for the call.
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
    // SAFETY: a `u64` buffer can always be read as bytes over the same length.
    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8) }
}

fn guid_text(guid: &GUID) -> String {
    let d4 = guid.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1, guid.data2, guid.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}

fn layout(handle: &Handle) -> io::Result<Vec<RawPartition>> {
    let buffer = ioctl(handle, IOCTL_DISK_GET_DRIVE_LAYOUT_EX, &[], 16 * 1024)?;
    let info = buffer.as_ptr().cast::<DRIVE_LAYOUT_INFORMATION_EX>();
    // SAFETY: the call succeeded and wrote the header into this aligned buffer.
    let header = unsafe { &*info };
    let count = usize::try_from(header.PartitionCount).unwrap_or(0);
    let room = (buffer.len() * 8 - size_of::<DRIVE_LAYOUT_INFORMATION_EX>())
        / size_of::<PARTITION_INFORMATION_EX>()
        + 1;
    // SAFETY: `count` is bounded by what the buffer holds; the system wrote these entries.
    let entries =
        unsafe { std::slice::from_raw_parts(header.PartitionEntry.as_ptr(), count.min(room)) };
    Ok(entries
        .iter()
        // With MBR, Windows also lists empty entries numbered 0.
        .filter(|entry| entry.PartitionNumber != 0 && entry.PartitionLength > 0)
        .map(|entry| RawPartition {
            number: entry.PartitionNumber,
            offset: u64::try_from(entry.StartingOffset).unwrap_or(0),
            size_bytes: u64::try_from(entry.PartitionLength).unwrap_or(0),
            // SAFETY: the entry style gives the valid union member.
            gpt_type: (entry.PartitionStyle == PARTITION_STYLE_GPT)
                .then(|| guid_text(unsafe { &entry.Anonymous.Gpt.PartitionType })),
            mbr_type: if entry.PartitionStyle == PARTITION_STYLE_MBR {
                // SAFETY: the entry style gives the valid union member.
                Some(unsafe { entry.Anonymous.Mbr.PartitionType })
            } else {
                None
            },
        })
        .collect())
}

/// `None` if it does not exist or has no medium (empty card reader).
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
    // SAFETY: plain struct read as bytes over its own size.
    let input = unsafe {
        std::slice::from_raw_parts(
            (&raw const query).cast::<u8>(),
            size_of::<STORAGE_PROPERTY_QUERY>(),
        )
    };
    let property = ioctl(&handle, IOCTL_STORAGE_QUERY_PROPERTY, input, 4096)?;
    // SAFETY: the call succeeded and wrote the descriptor into this aligned buffer.
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
    // SAFETY: the call succeeded and wrote the geometry into this aligned buffer.
    let (size, bytes_per_sector) = unsafe {
        let geometry = &*geometry.as_ptr().cast::<DISK_GEOMETRY_EX>();
        (geometry.DiskSize, geometry.Geometry.BytesPerSector)
    };
    let partitions = layout(&handle)?;
    Ok(Some(RawDisk {
        number,
        bus_type: descriptor.BusType,
        removable: descriptor.RemovableMedia,
        vendor: text_at(raw, descriptor.VendorIdOffset),
        product: text_at(raw, descriptor.ProductIdOffset),
        serial: text_at(raw, descriptor.SerialNumberOffset),
        size_bytes: u64::try_from(size).unwrap_or(0),
        block_size: (bytes_per_sector > 0).then_some(bytes_per_sector),
        instance,
        partitions,
    }))
}

/// Descriptive: without it the id falls back to the number.
fn disk_instances() -> HashMap<u32, String> {
    let mut found = HashMap::new();
    let class = GUID_DEVINTERFACE_DISK;
    // SAFETY: valid GUID; the other arguments are optional.
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
        // SAFETY: plain struct; its size is set as the API requires.
        let mut interface: SP_DEVICE_INTERFACE_DATA = unsafe { std::mem::zeroed() };
        interface.cbSize = u32::try_from(size_of::<SP_DEVICE_INTERFACE_DATA>()).unwrap_or(0);
        // SAFETY: valid set, valid output.
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
            // SAFETY: the call succeeded and wrote the number into this aligned buffer.
            let number = unsafe { (*number.as_ptr().cast::<STORAGE_DEVICE_NUMBER>()).DeviceNumber };
            found.insert(number, path);
        }
    }
    // SAFETY: valid set, closed once.
    unsafe {
        SetupDiDestroyDeviceInfoList(set);
    }
    found
}

fn interface_path(set: HDEVINFO, interface: &SP_DEVICE_INTERFACE_DATA) -> Option<String> {
    // Aligned buffer: the struct header precedes the UTF-16 path.
    let mut buffer = vec![0u64; 512];
    let detail = buffer
        .as_mut_ptr()
        .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
    let capacity = u32::try_from(buffer.len() * 8).ok()?;
    // SAFETY: large enough and aligned; `cbSize` is the header's, as the API requires.
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
        // SAFETY: zero-terminated name, buffer valid for the declared length.
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

/// `None` for a volume without a disk (optical drive, empty).
fn extents(volume: &str) -> io::Result<Option<Vec<RawExtent>>> {
    // Without the trailing backslash: with it the name means the root folder, not the volume.
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
    // SAFETY: the call succeeded and wrote the header into this aligned buffer.
    let header = unsafe { &*buffer.as_ptr().cast::<VOLUME_DISK_EXTENTS>() };
    let count = usize::try_from(header.NumberOfDiskExtents).unwrap_or(0);
    let room = (buffer.len() * 8 - size_of::<VOLUME_DISK_EXTENTS>()) / size_of::<DISK_EXTENT>() + 1;
    // SAFETY: `count` is bounded by what the buffer holds; the system wrote these entries.
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

/// Descriptive: an unreadable volume simply has none.
fn volume_information(volume: &str) -> (Option<String>, Option<String>) {
    let name = wide(volume);
    let mut label = [0u16; 261];
    let mut file_system = [0u16; 261];
    // SAFETY: zero-terminated name, buffers valid for the declared lengths, other outputs optional.
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
    // SAFETY: buffer valid for the declared length.
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
        // SAFETY: `find` is valid until `FindVolumeClose`; buffer valid for the declared length.
        if unsafe { FindNextVolumeW(find, name.as_mut_ptr(), 64) } == 0 {
            let error = io::Error::last_os_error();
            break if has_code(&error, &[ERROR_NO_MORE_FILES]) {
                Ok(())
            } else {
                Err(error)
            };
        }
    };
    // SAFETY: `find` comes from `FindFirstVolumeW` and is closed only here.
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
