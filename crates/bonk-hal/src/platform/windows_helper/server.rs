use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr::null_mut;

use bonk_core::format::Filesystem;
use serde_json::{Value, json};
use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, PROCESS_DUP_HANDLE};

use crate::image::Access;
use crate::platform::Platform;
use crate::platform::windows::Windows;
use crate::platform::windows_disk;
use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};
use crate::windows_facts::partition_numbers;

/// With the widest options: the client's request is not trusted, but it may have allowed an internal or
/// virtual disk. The system disk and its kind stay refused regardless.
///
/// `fresh_table`: Bonk just wrote the table, and Windows may have given the new partition a letter. That
/// mount says nothing about the disk.
pub(in crate::platform) fn check_allowed(path: &str, fresh_table: bool) -> Result<(), String> {
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
pub(in crate::platform) fn hand_over(file: &File, client: u32) -> io::Result<usize> {
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

pub(in crate::platform) fn respond(request: &Value, client: u32) -> Result<Value, String> {
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
pub(in crate::platform) fn serve(client: u32, pipe: &str) -> Result<(), String> {
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
