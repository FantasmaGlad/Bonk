use std::collections::HashMap;
use std::io;
use std::mem::offset_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_NO_MEDIA_IN_DRIVE, ERROR_NOT_READY, ERROR_PATH_NOT_FOUND,
};
use windows_sys::Win32::System::Ioctl::{
    DISK_GEOMETRY_EX, DRIVE_LAYOUT_INFORMATION_EX, GUID_DEVINTERFACE_DISK,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_GET_DRIVE_LAYOUT_EX,
    IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_QUERY_PROPERTY, PARTITION_INFORMATION_EX,
    PARTITION_STYLE_GPT, PARTITION_STYLE_MBR, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_DEVICE_NUMBER, STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};

use super::ioctl::Handle;
use crate::platform::windows_query::{
    as_bytes, from_wide, guid_text, has_code, ioctl, open_for_query, text_at,
};
use crate::windows_facts::{RawDisk, RawPartition};

pub(super) fn layout(handle: &Handle) -> io::Result<Vec<RawPartition>> {
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

/// Size and sector size; `None` without a medium (empty card reader).
fn geometry(handle: &Handle) -> io::Result<Option<(i64, u32)>> {
    let geometry = match ioctl(
        handle,
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
    let geometry = unsafe { &*geometry.as_ptr().cast::<DISK_GEOMETRY_EX>() };
    Ok(Some((geometry.DiskSize, geometry.Geometry.BytesPerSector)))
}

/// `None` if it does not exist or has no medium (empty card reader).
pub(super) fn read_disk(number: u32, instance: Option<String>) -> io::Result<Option<RawDisk>> {
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

    let Some((size, bytes_per_sector)) = geometry(&handle)? else {
        return Ok(None);
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
pub(super) fn disk_instances() -> HashMap<u32, String> {
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

pub(super) fn interface_path(
    set: HDEVINFO,
    interface: &SP_DEVICE_INTERFACE_DATA,
) -> Option<String> {
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
