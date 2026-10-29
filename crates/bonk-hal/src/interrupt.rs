//! Demande d'annulation par Ctrl+C (CDC §3.4 : annulation propre possible).
//!
//! Le gestionnaire de signal ne fait que lever un drapeau atomique, seule opération permise dans un
//! gestionnaire de signal. Le moteur d'écriture le consulte entre deux blocs et s'arrête proprement :
//! il n'est jamais interrompu au milieu d'un bloc. Sous Unix seulement ; ailleurs le drapeau n'est
//! jamais levé (l'annulation passe alors par les autres canaux, comme le service).

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

/// Une annulation a-t-elle été demandée depuis [`install`] ?
#[must_use]
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
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
