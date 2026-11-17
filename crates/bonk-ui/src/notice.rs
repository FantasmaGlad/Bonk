use bonk_hal::notify::{Notice, Step};

use crate::browse::DownloadState;
use crate::event::Outcome;
use crate::model::{Model, WriteState};

/// The error code of a cancelled operation.
const CANCELLED: &str = "E4003";

impl Model {
    /// What the system's notification says now; `None` when the setting is off or nothing ran.
    #[must_use]
    pub fn notice(&self) -> Option<Notice> {
        if !self.notify {
            return None;
        }
        let step = match (&self.write, &self.browse.download) {
            (
                WriteState::Running {
                    phase, done, total, ..
                },
                _,
            ) => Some(Step::Progress {
                phase: *phase,
                done: *done,
                total: *total,
            }),
            (
                _,
                Some(DownloadState::Running {
                    step: Some(step), ..
                }),
            ) => Some(Step::Progress {
                phase: step.phase,
                done: step.done,
                total: step.total,
            }),
            (WriteState::Finished { outcome, .. }, _)
            | (_, Some(DownloadState::Finished { outcome, .. })) => ended(outcome),
            _ => None,
        }?;
        Some(Notice::new(&step, self.lang))
    }
}

fn ended(outcome: &Outcome) -> Option<Step> {
    Some(match outcome {
        Outcome::Done { verified, .. } => Step::Written {
            verified: *verified,
        },
        Outcome::Downloaded { path, .. } => Step::Downloaded { path: path.clone() },
        Outcome::Failed {
            code: Some(code), ..
        } if code == CANCELLED => Step::Cancelled,
        Outcome::Failed { message, .. } => Step::Failed {
            message: message.clone(),
        },
        Outcome::Vault(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use bonk_core::write::Phase;
    use bonk_hal::Lang;

    use super::*;
    use crate::command::ViewOptions;

    fn model(notify: bool) -> Model {
        let options = ViewOptions {
            notify,
            ..ViewOptions::default()
        };
        Model::new(options, Lang::Fr).0
    }

    fn running(phase: Phase, done: u64) -> WriteState {
        WriteState::Running {
            disk_id: "usb-a".to_owned(),
            pid: None,
            phase,
            done,
            total: Some(100),
            rate: 0.0,
        }
    }

    #[test]
    fn chaque_etape_de_l_ecriture_donne_sa_notification() {
        let mut model = model(true);
        assert_eq!(model.notice(), None, "rien ne tourne");
        for (phase, title) in [
            (Phase::Download, "Téléchargement"),
            (Phase::Source, "Contrôle de l'image"),
            (Phase::Write, "Écriture"),
            (Phase::Verify, "Vérification"),
        ] {
            model.write = running(phase, 40);
            let notice = model.notice();
            assert_eq!(
                notice.as_ref().map(|n| (n.title.as_str(), n.percent)),
                Some((title, Some(40)))
            );
        }
    }

    #[test]
    fn fin_echec_et_annulation() {
        let mut model = model(true);
        let finished = |outcome| WriteState::Finished {
            disk_id: "usb-a".to_owned(),
            outcome,
        };
        model.write = finished(Outcome::Done {
            bytes: 1,
            verified: true,
            gpt_repaired: false,
            secure_boot: None,
            bios_boot: false,
        });
        assert_eq!(
            model.notice().map(|n| n.title),
            Some("Clé prête".to_owned())
        );
        model.write = finished(Outcome::Failed {
            code: Some(CANCELLED.to_owned()),
            message: "cancelled".to_owned(),
        });
        assert_eq!(model.notice().map(|n| n.title), Some("Annulé".to_owned()));
        model.write = finished(Outcome::Failed {
            code: Some("E4002".to_owned()),
            message: "mismatch".to_owned(),
        });
        assert_eq!(
            model.notice().map(|n| (n.title, n.body)),
            Some(("Échec".to_owned(), "mismatch".to_owned()))
        );
    }

    #[test]
    fn reglage_coupe_rien_n_est_notifie() {
        let mut model = model(false);
        model.write = running(Phase::Write, 10);
        assert_eq!(model.notice(), None);
    }
}
