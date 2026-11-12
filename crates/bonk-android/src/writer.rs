use std::path::Path;
use std::sync::atomic::Ordering;

use bonk_core::format::table::TableKind;
use bonk_core::write::{Control, Progress};
use bonk_core::{BlockDevice, BlockError};
use bonk_hal::{Disk, Elevation, FilterOptions};
use bonk_ops::job::{self, Deps, Io, Target, WriteJob, WriteMode, WriteOutcome};
use bonk_ui::event::{Event, Outcome, parse_line};
use serde::Serialize;

use crate::screen::{CANCEL, EVENTS, MODEL, lock};

pub(crate) fn push(event: Event) {
    lock(&EVENTS).push_back(event);
}

fn unavailable(what: &str) -> BlockError {
    BlockError::Io(std::io::Error::other(format!(
        "{what}: not available on Android"
    )))
}

/// `open` opens the key the app handed over. No unmount and no table reread: Android detached its driver when
/// handing the key, and takes it back when the app returns it.
pub(crate) fn android_io(
    open: impl Fn() -> Result<Box<dyn BlockDevice>, BlockError> + 'static,
) -> Io {
    Io {
        prepare: Box::new(|_, _, _| Ok(())),
        unmount: Box::new(|_, _| Ok(Vec::new())),
        open_node: Box::new(move |_, _, _| open()),
        cancelled: Box::new(|| CANCEL.load(Ordering::SeqCst)),
        capabilities: Box::new(Vec::new),
        format_partition: Box::new(|_, _, _, _| Err(unavailable("host format"))),
        rescan: Box::new(|_, _| Ok(())),
        pause: Box::new(|| std::thread::sleep(std::time::Duration::from_millis(200))),
    }
}

/// In the command line format: one reader, as on the desktop.
pub(crate) fn finished<T: Serialize>(result: Result<T, job::JobError>) -> Event {
    let failure =
        |message: String, code: Option<String>| Event::Finished(Outcome::Failed { code, message });
    match result {
        Err(error) => failure(error.message, Some(error.code.id().to_owned())),
        Ok(done) => serde_json::to_string(&done)
            .ok()
            .and_then(|line| parse_line(&line))
            .unwrap_or_else(|| failure("unreadable result".to_owned(), None)),
    }
}

/// Engine progress to the screen; the screen's cancel stops the engine.
pub(crate) fn report(update: &Progress) -> Control {
    push(Event::Progress {
        phase: update.phase,
        done: update.bytes_done,
        total: update.bytes_total,
        rate: update.bytes_per_second,
    });
    if CANCEL.load(Ordering::SeqCst) {
        Control::Cancel
    } else {
        Control::Continue
    }
}

pub(crate) struct Request<'a> {
    pub(crate) disk_id: &'a str,
    /// What the app really opened: USB device name and its serial (empty without one).
    pub(crate) device: &'a str,
    pub(crate) serial: &'a str,
    pub(crate) source: &'a str,
    pub(crate) sha256: Option<String>,
    pub(crate) raw: bool,
    pub(crate) mbr: bool,
}

/// The engine re-check compares the list with itself on Android (the enumeration is the screen's): only this
/// comparison ties the chosen disk to the device the app just opened.
pub(crate) fn same_drive(
    disks: &[Disk],
    disk_id: &str,
    device: &str,
    serial: &str,
) -> Result<(), String> {
    let chosen = disks
        .iter()
        .find(|disk| disk.id == disk_id)
        .ok_or_else(|| format!("{disk_id} is no longer connected"))?;
    if chosen.path != device || chosen.serial.as_deref().unwrap_or("") != serial {
        return Err(format!(
            "{disk_id} was chosen at {}, serial {}; the drive opened is at {device}, serial {}",
            chosen.path,
            chosen.serial.as_deref().unwrap_or("none"),
            if serial.is_empty() { "none" } else { serial },
        ));
    }
    Ok(())
}

/// The app shows no expert option: verification and GPT repair always run.
fn write_job(request: &Request<'_>) -> WriteJob {
    WriteJob {
        target: Target::Disk(request.disk_id.to_owned()),
        source: Path::new(request.source).to_path_buf(),
        options: FilterOptions::default(),
        verify: true,
        repair_gpt: true,
        trace_io: None,
        dry_run: false,
        yes: true,
        expected_sha256: request.sha256.clone(),
        mode: if request.raw {
            WriteMode::Raw
        } else {
            WriteMode::Auto
        },
        table: if request.mbr {
            TableKind::Mbr
        } else {
            TableKind::Gpt
        },
        elevation: Elevation::Never,
        force: false,
    }
}

/// Writes and verifies end to end. Blocks: the app calls it off the UI thread and reads progress elsewhere.
pub(crate) fn write(
    request: &Request<'_>,
    open: impl Fn() -> Result<Box<dyn BlockDevice>, BlockError> + 'static,
) {
    let disks: Vec<Disk> = lock(&MODEL).disks.clone();
    if let Err(message) = same_drive(&disks, request.disk_id, request.device, request.serial) {
        push(Event::Finished(Outcome::Failed {
            code: Some(bonk_ops::errors::Code::IdentityChanged.id().to_owned()),
            message,
        }));
        return;
    }
    let io = android_io(open);
    let enumerate = move || Ok(disks.clone());
    let deps = Deps {
        enumerate: &enumerate,
        io: &io,
    };
    let job = write_job(request);
    // Null when unserializable: read back as an unreadable result.
    let result = job::write(&job, &deps, &mut report).map(|outcome| {
        match outcome {
            WriteOutcome::Done(done) => serde_json::to_value(done),
            WriteOutcome::Extracted(done) => serde_json::to_value(done),
            WriteOutcome::Plan(plan) => serde_json::to_value(plan),
        }
        .unwrap_or_default()
    });
    push(finished(result));
}

#[cfg(test)]
mod tests {
    use bonk_hal::{Bus, DiskKind};

    use super::*;

    fn disk(path: &str, serial: Option<&str>) -> Disk {
        Disk {
            id: format!("usb-{}", serial.unwrap_or(path)),
            path: path.to_owned(),
            model: "UDisk".to_owned(),
            serial: serial.map(str::to_owned),
            size_bytes: 32_000_000_000,
            bus: Bus::Usb,
            kind: DiskKind::Physical,
            removable_flag: true,
            logical_block_size: None,
            location: None,
            fs_type: None,
            holders: Vec::new(),
            partitions: Vec::new(),
        }
    }

    /// The key the app opened must be the one the screen showed: same plug point, same serial.
    #[test]
    fn la_cle_ouverte_doit_etre_celle_choisie() {
        let disks = [disk("/dev/bus/usb/001/002", Some("S1"))];
        assert!(same_drive(&disks, "usb-S1", "/dev/bus/usb/001/002", "S1").is_ok());
        // Another key at the same place, or the chosen key replugged elsewhere.
        assert!(same_drive(&disks, "usb-S1", "/dev/bus/usb/001/002", "S2").is_err());
        assert!(same_drive(&disks, "usb-S1", "/dev/bus/usb/001/003", "S1").is_err());
        // The chosen disk is no longer in the list.
        assert!(same_drive(&disks, "usb-S9", "/dev/bus/usb/001/002", "S1").is_err());
        // Without a serial, the plug point alone tells them apart.
        let anonymous = [disk("/dev/bus/usb/001/004", None)];
        assert!(
            same_drive(
                &anonymous,
                "usb-/dev/bus/usb/001/004",
                "/dev/bus/usb/001/004",
                ""
            )
            .is_ok()
        );
    }
}
