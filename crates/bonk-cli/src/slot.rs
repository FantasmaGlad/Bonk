//! Emplacement unique de l'opération longue du service `bonk serve`.
//!
//! Invariant de protocole : une opération terminée est immédiatement remplaçable. Un client qui envoie
//! la requête suivante dès qu'il a reçu la réponse ne doit jamais être refusé. L'emplacement est donc
//! rendu **avant** l'envoi de la réponse, et jamais plus tard que la fin du fil, panique comprise.
//! Isolé ici pour être éprouvé sans fil ni ordonnanceur.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
pub(crate) struct JobSlot(Arc<AtomicBool>);

impl JobSlot {
    pub(crate) fn is_busy(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// Prend l'emplacement s'il est libre, en une seule opération atomique.
    pub(crate) fn try_acquire(&self) -> Option<SlotGuard> {
        self.0
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| SlotGuard(Arc::clone(&self.0)))
    }
}

/// Possession de l'emplacement ; rendu à la destruction, y compris si l'opération panique.
pub(crate) struct SlotGuard(Arc<AtomicBool>);

impl SlotGuard {
    /// Rend l'emplacement, puis exécute `respond` : c'est l'ordre qui garantit l'invariant.
    pub(crate) fn release_then<R>(self, respond: impl FnOnce() -> R) -> R {
        drop(self);
        respond()
    }
}

impl Drop for SlotGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    type Resultat = Result<(), &'static str>;

    #[test]
    fn un_emplacement_pris_refuse_le_suivant() {
        let slot = JobSlot::default();
        let guard = slot.try_acquire();
        assert!(guard.is_some() && slot.is_busy());
        assert!(slot.try_acquire().is_none());
    }

    #[test]
    fn l_emplacement_est_rendu_a_la_destruction() {
        let slot = JobSlot::default();
        drop(slot.try_acquire());
        assert!(!slot.is_busy());
        assert!(slot.try_acquire().is_some());
    }

    #[test]
    fn la_reponse_part_apres_la_liberation() -> Resultat {
        let slot = JobSlot::default();
        let guard = slot.try_acquire().ok_or("emplacement libre attendu")?;
        // Le client réagit à la réponse en redemandant l'emplacement : il doit l'obtenir.
        let obtenu = guard.release_then(|| slot.try_acquire().is_some());
        assert!(obtenu);
        Ok(())
    }

    #[test]
    fn une_operation_qui_panique_rend_l_emplacement() -> Resultat {
        let slot = JobSlot::default();
        let guard = slot.try_acquire().ok_or("emplacement libre attendu")?;
        let fil = std::thread::spawn(move || {
            let _possede = guard;
            panic!("opération interrompue");
        });
        assert!(fil.join().is_err());
        assert!(!slot.is_busy());
        Ok(())
    }

    #[test]
    fn une_seule_requete_simultanee_obtient_l_emplacement() -> Resultat {
        let slot = JobSlot::default();
        let depart = Arc::new(Barrier::new(8));
        let fils: Vec<_> = (0..8)
            .map(|_| {
                let (slot, depart) = (slot.clone(), Arc::clone(&depart));
                std::thread::spawn(move || {
                    depart.wait();
                    slot.try_acquire()
                })
            })
            .collect();
        // Les possessions restent vivantes jusqu'à la fin du comptage : un perdant en retard ne doit pas
        // trouver l'emplacement rendu par le gagnant.
        let mut resultats = Vec::new();
        for fil in fils {
            resultats.push(fil.join().map_err(|_| "fil en panique")?);
        }
        let gagnants = resultats.iter().filter(|r| r.is_some()).count();
        assert_eq!(gagnants, 1);
        Ok(())
    }
}
