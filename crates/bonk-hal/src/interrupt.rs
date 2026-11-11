use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_signal: libc::c_int) {
    REQUESTED.store(true, Ordering::SeqCst);
}

#[cfg(windows)]
unsafe extern "system" fn on_console_event(_event: u32) -> windows_sys::core::BOOL {
    REQUESTED.store(true, Ordering::SeqCst);
    1
}

#[cfg(windows)]
fn event_name(pid: u32) -> Vec<u16> {
    format!("Local\\bonk-stop-{pid}\0").encode_utf16().collect()
}

/// Created once and kept until exit. Stored as an integer: a raw handle pointer cannot be shared between
/// threads otherwise.
#[cfg(windows)]
static STOP_EVENT: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

#[cfg(windows)]
fn install_windows() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent};
    // SAFETY: the handler only does an atomic store.
    let installed = unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) };
    if installed == 0 {
        log::warn!(target: "bonk::hal", "Ctrl+C handler not installed: {}", std::io::Error::last_os_error());
    }
    let handle = *STOP_EVENT.get_or_init(|| {
        let name = event_name(std::process::id());
        // SAFETY: zero-terminated name; default security attributes.
        unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) as isize }
    });
    if handle == 0 {
        log::warn!(target: "bonk::hal", "stop event not created: {}", std::io::Error::last_os_error());
    } else {
        // SAFETY: valid handle, kept until exit.
        unsafe {
            ResetEvent(handle as windows_sys::Win32::Foundation::HANDLE);
        }
    }
}

/// Call at the start of a write operation.
pub fn install() {
    REQUESTED.store(false, Ordering::SeqCst);
    #[cfg(windows)]
    install_windows();
    #[cfg(unix)]
    {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SAFETY: the handler only does an atomic store, which is signal-safe.
        unsafe {
            libc::signal(libc::SIGINT, handler);
            libc::signal(libc::SIGTERM, handler);
        }
    }
}

/// Asks another process to stop cleanly; it acts between two blocks.
///
/// # Errors
/// `Unsupported` on a system without a way to stop it, so the interface says so instead of seeming to cancel.
/// Elsewhere: the process is gone or belongs to another user.
pub fn stop_process(pid: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let pid = i32::try_from(pid).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        // SAFETY: only sends SIGTERM, no memory involved.
        if unsafe { libc::kill(pid, libc::SIGTERM) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};
        let name = event_name(pid);
        // SAFETY: zero-terminated name.
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if event.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: valid handle just opened, closed once.
        let done = unsafe {
            let done = SetEvent(event);
            CloseHandle(event);
            done
        };
        if done == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

#[must_use]
pub fn requested() -> bool {
    #[cfg(windows)]
    if let Some(&handle) = STOP_EVENT.get()
        && handle != 0
    {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY: handle valid until exit; zero wait.
        let state =
            unsafe { WaitForSingleObject(handle as windows_sys::Win32::Foundation::HANDLE, 0) };
        if state == WAIT_OBJECT_0 {
            return true;
        }
    }
    REQUESTED.load(Ordering::SeqCst)
}

#[cfg(all(test, windows))]
mod tests_windows {
    use super::*;

    #[test]
    fn l_evenement_nomme_leve_la_demande() -> std::io::Result<()> {
        install();
        assert!(!requested());
        stop_process(std::process::id())?;
        assert!(requested());
        install();
        assert!(!requested());
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn un_signal_leve_le_drapeau() {
        install();
        assert!(!requested());
        // SAFETY: sends SIGTERM to itself, whose handler was just installed.
        unsafe {
            libc::raise(libc::SIGTERM);
        }
        assert!(requested());
        // Installing again resets the flag for the next operation.
        install();
        assert!(!requested());
    }
}
