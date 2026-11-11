use std::path::Path;
use std::process::{Command, Stdio};

/// Through the command line, the configuration file's only owner. False when it could not be kept.
pub(super) fn save(program: &Path, key: &str, value: &str) -> bool {
    let mut command = Command::new(program);
    bonk_hal::console::hide_child_console(&mut command);
    let saved = command
        .args(["config", "set", key, value])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|output| output.status.success());
    if !saved {
        log::warn!(target: "bonk::gui", key = key; "setting not saved");
    }
    saved
}

/// The same program with the same arguments: it reads the new setting at start. False when it could not
/// start, and this window then stays open.
pub(super) fn relaunch() -> bool {
    let Ok(program) = std::env::current_exe() else {
        log::warn!(target: "bonk::gui", "relaunch impossible: program path unknown");
        return false;
    };
    let mut command = Command::new(program);
    command
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::null());
    match command.spawn() {
        Ok(_) => true,
        Err(error) => {
            log::warn!(target: "bonk::gui", error:% = error; "relaunch failed");
            false
        }
    }
}
