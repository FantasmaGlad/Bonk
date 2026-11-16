use std::ffi::{OsString, c_void};
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::core::GUID;

pub(in crate::platform) struct Handle(pub(in crate::platform) HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle comes from `CreateFileW`, valid, and is closed only here.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(in crate::platform) fn wide(text: &str) -> Vec<u16> {
    std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(Some(0))
        .collect()
}

pub(super) fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..end])
        .to_string_lossy()
        .into_owned()
}

pub(super) fn open_for_query(path: &str) -> io::Result<Handle> {
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

pub(super) fn has_code(error: &io::Error, codes: &[u32]) -> bool {
    error
        .raw_os_error()
        .and_then(|code| u32::try_from(code).ok())
        .is_some_and(|code| codes.contains(&code))
}

pub(in crate::platform) fn ioctl_raw(
    handle: HANDLE,
    code: u32,
    capacity: usize,
) -> io::Result<Vec<u64>> {
    ioctl_on(handle, code, &[], capacity)
}

pub(super) fn ioctl(
    handle: &Handle,
    code: u32,
    input: &[u8],
    capacity: usize,
) -> io::Result<Vec<u64>> {
    ioctl_on(handle.0, code, input, capacity)
}

pub(super) fn ioctl_on(
    handle: HANDLE,
    code: u32,
    input: &[u8],
    capacity: usize,
) -> io::Result<Vec<u64>> {
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

pub(super) fn text_at(raw: &[u8], offset: u32) -> Option<String> {
    let offset = usize::try_from(offset).ok().filter(|&o| o != 0)?;
    let tail = raw.get(offset..)?;
    let end = tail.iter().position(|&b| b == 0).unwrap_or(tail.len());
    let text = String::from_utf8_lossy(&tail[..end]).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

pub(super) fn as_bytes(buffer: &[u64]) -> &[u8] {
    // SAFETY: a `u64` buffer can always be read as bytes over the same length.
    unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8) }
}

pub(super) fn guid_text(guid: &GUID) -> String {
    let d4 = guid.data4;
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1, guid.data2, guid.data3, d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]
    )
}
