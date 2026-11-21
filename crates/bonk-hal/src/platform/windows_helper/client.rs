use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::{null, null_mut};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_core::format::Filesystem;
use bonk_core::{BlockDevice, BlockError};
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{
    ERROR_CANCELLED, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{GetProcessId, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use crate::disk::Disk;
use crate::image::Access;
use crate::platform::windows_disk;
use crate::platform::windows_query::wide;
use crate::target::OpenTargetError;

use super::*;

pub(in crate::platform) struct Session {
    pipe: File,
    reader: BufReader<File>,
}

pub(in crate::platform) static SESSION: Mutex<Option<Session>> = Mutex::new(None);

pub(in crate::platform) fn start() -> io::Result<Session> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let client = std::process::id();
    let name = format!(r"\\.\pipe\bonk-helper-{client}-{nanos}");
    let pipe = create_pipe(&name)?;
    let process = launch_helper(&name, client)?;
    // SAFETY: valid handle from `ShellExecuteExW`.
    let helper = unsafe { GetProcessId(process) };
    // A helper that dies before connecting would leave `ConnectNamedPipe` blocked: this thread unblocks it,
    // and the process id check rejects that connection.
    let (watched, watched_name) = (process as isize, name.clone());
    std::thread::spawn(move || {
        // SAFETY: handle kept open until exit, never closed here.
        unsafe {
            WaitForSingleObject(watched as HANDLE, INFINITE);
        }
        let _ = OpenOptions::new().read(true).open(watched_name);
    });
    accept(&pipe, helper)?;
    Ok(Session {
        reader: BufReader::new(pipe.try_clone()?),
        pipe,
    })
}

pub(in crate::platform) fn create_pipe(name: &str) -> io::Result<File> {
    let wide_name = wide(name);
    // SAFETY: zero-terminated name; `FIRST_PIPE_INSTANCE` refuses a pipe someone else created.
    let pipe = unsafe {
        CreateNamedPipeW(
            wide_name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            null(),
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: valid handle, now solely owned by this `File`.
    Ok(unsafe { File::from_raw_handle(pipe) })
}

/// The helper is this executable, started with the `runas` verb: the system asks for the rights.
pub(in crate::platform) fn launch_helper(name: &str, client: u32) -> io::Result<HANDLE> {
    let exe = wide(&std::env::current_exe()?.to_string_lossy());
    let verb = wide("runas");
    let parameters = wide(&format!(
        "{HELPER_COMMAND} --client-pid {client} --pipe {name}"
    ));
    // SAFETY: zeroed plain struct, then fields valid for the whole call.
    let process = unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = u32::try_from(size_of::<SHELLEXECUTEINFOW>()).unwrap_or(0);
        info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = exe.as_ptr();
        info.lpParameters = parameters.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&raw mut info) == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == i32::try_from(ERROR_CANCELLED).ok() {
                return Err(denied("authorization cancelled"));
            }
            return Err(error);
        }
        info.hProcess
    };
    if process.is_null() {
        return Err(denied("the system started no helper"));
    }
    Ok(process)
}

/// Only the helper just started may hold the pipe.
pub(in crate::platform) fn accept(pipe: &File, helper: u32) -> io::Result<()> {
    // SAFETY: valid pipe handle, not overlapped.
    if unsafe { ConnectNamedPipe(pipe.as_raw_handle(), null_mut()) } == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != i32::try_from(ERROR_PIPE_CONNECTED).ok() {
            return Err(error);
        }
    }
    let mut peer = 0u32;
    // SAFETY: valid pipe, valid output.
    let known = unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle(), &raw mut peer) };
    if known == 0 || peer != helper {
        return Err(denied(
            "the helper did not start, or another process connected",
        ));
    }
    Ok(())
}

pub(in crate::platform) fn ask(request: &Value) -> io::Result<Value> {
    let mut slot = SESSION
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(start()?);
    }
    let session = slot
        .as_mut()
        .ok_or_else(|| io::Error::other("no helper session"))?;
    writeln!(session.pipe, "{request}")?;
    let mut line = String::new();
    if session.reader.read_line(&mut line)? == 0 {
        return Err(io::Error::other("the helper stopped"));
    }
    let reply: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
    match reply["error"].as_str() {
        Some(message) if reply["denied"] == true => Err(denied(message)),
        Some(message) => Err(io::Error::other(message.to_owned())),
        None => Ok(reply),
    }
}

pub(in crate::platform) fn prepare(path: &Path, access: Access) -> Result<(), BlockError> {
    ask(&json!({"op": "check", "path": path.to_string_lossy(), "access": access_name(access)}))?;
    Ok(())
}

pub(in crate::platform) fn open(
    path: &Path,
    access: Access,
) -> Result<Box<dyn BlockDevice>, BlockError> {
    let reply =
        ask(&json!({"op": "open", "path": path.to_string_lossy(), "access": access_name(access)}))?;
    let handle = reply["handle"]
        .as_u64()
        .and_then(|h| usize::try_from(h).ok())
        .ok_or_else(|| io::Error::other("no handle in the helper reply"))?;
    // SAFETY: the helper just created this handle in this process, its sole owner.
    let file = File::from(unsafe { OwnedHandle::from_raw_handle(handle as *mut c_void) });
    Ok(Box::new(windows_disk::from_file(file, access)?))
}

pub(in crate::platform) fn unmount(disk: &Disk) -> Result<Vec<String>, OpenTargetError> {
    let reply = ask(&json!({"op": "unmount", "path": disk.path}))
        .map_err(|e| OpenTargetError::Unmount(e.to_string()))?;
    Ok(reply["unmounted"]
        .as_array()
        .map(|mounts| {
            mounts
                .iter()
                .filter_map(|m| m.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

pub(in crate::platform) fn rescan(path: &Path) -> Result<(), BlockError> {
    ask(&json!({"op": "rescan", "path": path.to_string_lossy()}))?;
    Ok(())
}

pub(in crate::platform) fn format(
    partition: &Path,
    fs: Filesystem,
    label: Option<&str>,
) -> Result<(), BlockError> {
    ask(
        &json!({"op": "format", "path": partition.to_string_lossy(), "fs": fs.name(), "label": label}),
    )?;
    Ok(())
}
