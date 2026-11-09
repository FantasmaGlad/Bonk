use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Default)]
pub(crate) struct JobSlot(Arc<AtomicBool>);

impl JobSlot {
    pub(crate) fn is_busy(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    /// In one atomic operation.
    pub(crate) fn try_acquire(&self) -> Option<SlotGuard> {
        self.0
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| SlotGuard(Arc::clone(&self.0)))
    }
}

/// Released on drop, even if the operation panics.
pub(crate) struct SlotGuard(Arc<AtomicBool>);

impl SlotGuard {
    /// Releases, then runs `respond`: this order is what guarantees the invariant.
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
        // The client reacts to the response by asking for the slot again: it must get it.
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
        // Guards stay alive until counting ends: a late loser must not find the slot the winner released.
        let mut resultats = Vec::new();
        for fil in fils {
            resultats.push(fil.join().map_err(|_| "fil en panique")?);
        }
        let gagnants = resultats.iter().filter(|r| r.is_some()).count();
        assert_eq!(gagnants, 1);
        Ok(())
    }
}
