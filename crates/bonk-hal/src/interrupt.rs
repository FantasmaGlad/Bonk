//! Demande d'annulation par Ctrl+C ou SIGTERM.
//!
//! Le gestionnaire de signal ne fait que lever un drapeau atomique, seule opération permise dans un
//! gestionnaire de signal. Le moteur d'écriture le consulte entre deux blocs : il n'est jamais
//! interrompu au milieu d'un bloc.
//!
//! Windows n'a pas de signal à envoyer à un autre processus sans console commune : la fenêtre, sans
//! console, arrête son sous-processus par un événement nommé d'après le numéro de celui-ci.

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

/// Événement d'arrêt de ce processus, créé une fois et gardé jusqu'à sa fin. Stocké en entier : une
/// poignée, pointeur brut, ne se partage pas entre fils sans cela.
#[cfg(windows)]
static STOP_EVENT: std::sync::OnceLock<isize> = std::sync::OnceLock::new();

#[cfg(windows)]
fn install_windows() {
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
    use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent};
    // SAFETY : le gestionnaire ne fait qu'une écriture atomique.
    let installed = unsafe { SetConsoleCtrlHandler(Some(on_console_event), 1) };
    if installed == 0 {
        log::warn!(target: "bonk::cli", "Ctrl+C handler not installed: {}", std::io::Error::last_os_error());
    }
    let handle = *STOP_EVENT.get_or_init(|| {
        let name = event_name(std::process::id());
        // SAFETY : nom terminé par un zéro ; attributs de sécurité par défaut.
        unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) as isize }
    });
    if handle == 0 {
        log::warn!(target: "bonk::cli", "stop event not created: {}", std::io::Error::last_os_error());
    } else {
        // SAFETY : poignée valide, gardée jusqu'à la fin du processus.
        unsafe {
            ResetEvent(handle as windows_sys::Win32::Foundation::HANDLE);
        }
    }
}

/// Installe le gestionnaire d'interruption. À appeler au début d'une opération d'écriture.
pub fn install() {
    REQUESTED.store(false, Ordering::SeqCst);
    #[cfg(windows)]
    install_windows();
    #[cfg(unix)]
    {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SAFETY : `signal` est appelée avec un gestionnaire qui ne fait qu'une écriture atomique,
        // opération sûre dans un gestionnaire de signal.
        unsafe {
            libc::signal(libc::SIGINT, handler);
            libc::signal(libc::SIGTERM, handler);
        }
    }
}

/// Demande à un autre processus de s'arrêter proprement : il traite la demande entre deux blocs.
///
/// # Errors
/// Sur un système sans moyen d'arrêt : `Unsupported`, pour que l'interface le dise au lieu de sembler
/// annuler. Ailleurs : le processus n'existe plus ou n'appartient pas à l'utilisateur.
pub fn stop_process(pid: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let pid = i32::try_from(pid).map_err(|_| std::io::ErrorKind::InvalidInput)?;
        // SAFETY : on n'envoie que SIGTERM, sans toucher à la mémoire.
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
        // SAFETY : nom terminé par un zéro.
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if event.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY : poignée valide juste ouverte, fermée une seule fois.
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

/// Une annulation a-t-elle été demandée depuis [`install`] ?
#[must_use]
pub fn requested() -> bool {
    #[cfg(windows)]
    if let Some(&handle) = STOP_EVENT.get()
        && handle != 0
    {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::WaitForSingleObject;
        // SAFETY : poignée valide jusqu'à la fin du processus ; attente nulle.
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

// Le test envoie un signal Unix : hors Unix, le module serait vide.
#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn un_signal_leve_le_drapeau() {
        install();
        assert!(!requested());
        // SAFETY : on s'envoie à soi-même SIGTERM, dont le gestionnaire vient d'être installé.
        unsafe {
            libc::raise(libc::SIGTERM);
        }
        assert!(requested());
        // Une nouvelle installation remet le drapeau à zéro pour l'opération suivante.
        install();
        assert!(!requested());
    }
}
