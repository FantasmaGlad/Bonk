use std::io;
use std::ptr::null_mut;

use windows_sys::Win32::Foundation::{
    ERROR_INVALID_FUNCTION, ERROR_MORE_DATA, ERROR_NO_MEDIA_IN_DRIVE, ERROR_NO_MORE_FILES,
    ERROR_NOT_READY, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    FindFirstVolumeW, FindNextVolumeW, FindVolumeClose, GetVolumeInformationW,
    GetVolumePathNamesForVolumeNameW, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::Ioctl::{DISK_EXTENT, VOLUME_DISK_EXTENTS};

use super::ioctl::wide;
use crate::platform::windows_query::{from_wide, has_code, ioctl, open_for_query};
use crate::windows_facts::{RawExtent, RawVolume};

pub(super) fn mount_points(volume: &str) -> io::Result<Vec<String>> {
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
pub(super) fn extents(volume: &str) -> io::Result<Option<Vec<RawExtent>>> {
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

pub(super) fn volume_information(volume: &str) -> (Option<String>, Option<String>) {
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

pub(in crate::platform) fn read_volumes() -> io::Result<Vec<RawVolume>> {
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
