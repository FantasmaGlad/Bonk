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
    DUPLICATE_SAME_ACCESS, DuplicateHandle, ERROR_CANCELLED, ERROR_PIPE_CONNECTED, HANDLE,
    INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessId, INFINITE, OpenProcess, PROCESS_DUP_HANDLE, WaitForSingleObject,
};
use windows_sys::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::Platform;
use super::windows::Windows;
use super::windows_disk;
use super::windows_query::wide;
use crate::disk::Disk;
use crate::image::Access;
use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};
use crate::target::OpenTargetError;
use crate::windows_facts::partition_numbers;

/// Hidden subcommand of the binary, started elevated.
const HELPER_COMMAND: &str = "elevated-helper";

fn access_name(access: Access) -> &'static str {
    match access {
        Access::ReadOnly => "ro",
        Access::ReadWrite => "rw",
    }
}

/// Client side: pipe to the helper, kept until exit.
struct Session {
    pipe: File,
    reader: BufReader<File>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn denied(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message.into())
}

fn start() -> io::Result<Session> {
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

fn create_pipe(name: &str) -> io::Result<File> {
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
fn launch_helper(name: &str, client: u32) -> io::Result<HANDLE> {
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
fn accept(pipe: &File, helper: u32) -> io::Result<()> {
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

/// Starts the helper if needed (one prompt per process), then sends the request.
fn ask(request: &Value) -> io::Result<Value> {
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

pub(super) fn prepare(path: &Path, access: Access) -> Result<(), BlockError> {
    ask(&json!({"op": "check", "path": path.to_string_lossy(), "access": access_name(access)}))?;
    Ok(())
}

pub(super) fn open(path: &Path, access: Access) -> Result<Box<dyn BlockDevice>, BlockError> {
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

pub(super) fn unmount(disk: &Disk) -> Result<Vec<String>, OpenTargetError> {
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

pub(super) fn rescan(path: &Path) -> Result<(), BlockError> {
    ask(&json!({"op": "rescan", "path": path.to_string_lossy()}))?;
    Ok(())
}

pub(super) fn format(
    partition: &Path,
    fs: Filesystem,
    label: Option<&str>,
) -> Result<(), BlockError> {
    ask(
        &json!({"op": "format", "path": partition.to_string_lossy(), "fs": fs.name(), "label": label}),
    )?;
    Ok(())
}

/// With the widest options: the client's request is not trusted, but it may have allowed an internal or
/// virtual disk. The system disk and its kind stay refused regardless.
///
/// `fresh_table`: Bonk just wrote the table, and Windows may have given the new partition a letter. That
/// mount says nothing about the disk.
fn check_allowed(path: &str, fresh_table: bool) -> Result<(), String> {
    let disks = Windows.enumerate().map_err(|e| e.to_string())?;
    let disk = disks
        .iter()
        .find(|d| d.path.eq_ignore_ascii_case(path))
        .ok_or_else(|| format!("disk not found: '{path}'"))?;
    let options = FilterOptions {
        show_internal: true,
        allow_virtual: true,
    };
    let refused = match evaluate(disk, options) {
        Eligibility::Excluded { reasons } => reasons
            .iter()
            .any(|r| !(fresh_table && matches!(r, Exclusion::MountedOutsideUserPaths { .. }))),
        _ => false,
    };
    if refused {
        return Err(format!("{path}: refused by the safety filter"));
    }
    Ok(())
}

/// The handle value only means something in the client.
fn hand_over(file: &File, client: u32) -> io::Result<usize> {
    // SAFETY: plain process id.
    let target = unsafe { OpenProcess(PROCESS_DUP_HANDLE, 0, client) };
    if target.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `target` was just opened and is closed only here.
    let target = unsafe { OwnedHandle::from_raw_handle(target) };
    let mut copy: HANDLE = null_mut();
    // SAFETY: valid handles; the copy belongs to the client.
    let done = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            file.as_raw_handle(),
            target.as_raw_handle(),
            &raw mut copy,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if done == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(copy as usize)
}

fn respond(request: &Value, client: u32) -> Result<Value, String> {
    let path = request["path"].as_str().ok_or("no path")?;
    let access = if request["access"] == "ro" {
        Access::ReadOnly
    } else {
        Access::ReadWrite
    };
    let text = |e: &dyn std::fmt::Display| e.to_string();
    match request["op"].as_str() {
        Some("check") => {
            check_allowed(path, false)?;
            windows_disk::prepare(Path::new(path), access).map_err(|e| text(&e))?;
            Ok(json!({}))
        }
        Some("unmount") => {
            check_allowed(path, false)?;
            let unmounted = windows_disk::unmount(path).map_err(|e| text(&e))?;
            Ok(json!({ "unmounted": unmounted }))
        }
        Some("open") => {
            check_allowed(path, false)?;
            let file = windows_disk::open_file(Path::new(path), access).map_err(|e| text(&e))?;
            let handle = hand_over(&file, client).map_err(|e| text(&e))?;
            Ok(json!({ "handle": handle }))
        }
        Some("rescan") => {
            check_allowed(path, true)?;
            windows_disk::rescan(Path::new(path)).map_err(|e| text(&e))?;
            Ok(json!({}))
        }
        Some("format") => {
            let (disk, _) = partition_numbers(path).ok_or("not a partition")?;
            check_allowed(&format!(r"\\.\PhysicalDrive{disk}"), true)?;
            let fs = Filesystem::ALL
                .into_iter()
                .find(|fs| request["fs"] == fs.name())
                .ok_or("unknown filesystem")?;
            windows_disk::format_partition(Path::new(path), fs, request["label"].as_str())
                .map_err(|e| text(&e))?;
            Ok(json!({}))
        }
        Some(other) => Err(format!("unknown request: {other}")),
        None => Err("no request".to_owned()),
    }
}

/// When the pipe closes the process exits and its locks go.
pub(super) fn serve(client: u32, pipe: &str) -> Result<(), String> {
    let text = |e: io::Error| e.to_string();
    let mut writer = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe)
        .map_err(text)?;
    let mut server = 0u32;
    // SAFETY: valid pipe, valid output.
    let known = unsafe { GetNamedPipeServerProcessId(writer.as_raw_handle(), &raw mut server) };
    if known == 0 || server != client {
        return Err("the pipe does not belong to the announced client".to_owned());
    }
    let reader = BufReader::new(writer.try_clone().map_err(text)?);
    for line in reader.lines() {
        let line = line.map_err(text)?;
        let reply = serde_json::from_str::<Value>(&line)
            .map_err(|e| e.to_string())
            .and_then(|request| respond(&request, client))
            .unwrap_or_else(|error| json!({ "error": error }));
        writeln!(writer, "{reply}").map_err(text)?;
    }
    Ok(())
}
