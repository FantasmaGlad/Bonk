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

mod disks;
mod ioctl;
mod volumes;

use disks::*;
use ioctl::*;
pub(super) use ioctl::{Handle, ioctl_raw, wide};
pub(super) use volumes::read_volumes;

/// Windows does not number disks beyond this in practice; numbers may have gaps.
const MAX_DISKS: u32 = 128;

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
