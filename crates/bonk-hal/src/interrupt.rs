//! Demande d'annulation par Ctrl+C.
//!
//! Le gestionnaire de signal ne fait que lever un drapeau atomique, seule opération permise dans un
//! gestionnaire de signal. Le moteur d'écriture le consulte entre deux blocs : il n'est jamais
//! interrompu au milieu d'un bloc. Sous Unix seulement ; ailleurs, le drapeau n'est jamais levé.

use std::sync::atomic::{AtomicBool, Ordering};

static REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_signal(_signal: libc::c_int) {
    REQUESTED.store(true, Ordering::SeqCst);
}

/// Installe le gestionnaire pour SIGINT et SIGTERM. À appeler au début d'une opération d'écriture.
pub fn install() {
    REQUESTED.store(false, Ordering::SeqCst);
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
/// Hors Unix : `Unsupported`, pour que l'interface le dise au lieu de sembler annuler. Sous Unix : le
/// processus n'existe plus ou n'appartient pas à l'utilisateur.
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
    #[cfg(not(unix))]
    {
        let _ = pid;
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Une annulation a-t-elle été demandée depuis [`install`] ?
#[must_use]
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

// Le seul test envoie un signal Unix : hors Unix, le module serait vide.
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
