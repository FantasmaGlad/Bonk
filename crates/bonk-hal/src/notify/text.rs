use bonk_core::write::Phase;

use super::{Notice, Step};
use crate::{Lang, format_size};

/// GNOME ignores the progress hint: the body carries the figure too.
pub(super) fn notice(step: &Step, lang: Lang) -> Notice {
    let (title, body, percent) = match step {
        Step::Progress { phase, done, total } => {
            let percent = total.filter(|total| *total > 0).map(|total| {
                u8::try_from(done.saturating_mul(100) / total)
                    .unwrap_or(100)
                    .min(100)
            });
            let body = match (percent, total) {
                (Some(percent), Some(total)) => {
                    format!(
                        "{percent} % · {} / {}",
                        format_size(*done, lang),
                        format_size(*total, lang)
                    )
                }
                _ => format_size(*done, lang),
            };
            (step_label(lang, *phase).to_owned(), body, percent)
        }
        Step::Written { verified } => (
            lang.pick("Drive ready", "Clé prête").to_owned(),
            if *verified {
                lang.pick("Written and verified.", "Écrite et relue.")
            } else {
                lang.pick("Written, not verified.", "Écrite, sans relecture.")
            }
            .to_owned(),
            Some(100),
        ),
        Step::Downloaded { path } => (
            lang.pick("Image downloaded", "Image téléchargée")
                .to_owned(),
            path.clone(),
            Some(100),
        ),
        Step::Failed { message } => (
            lang.pick("Failed", "Échec").to_owned(),
            first_line(message),
            None,
        ),
        Step::Cancelled => (
            lang.pick("Cancelled", "Annulé").to_owned(),
            lang.pick("Stopped at your request.", "Arrêté à votre demande.")
                .to_owned(),
            None,
        ),
    };
    Notice {
        title,
        body,
        percent,
        last: !matches!(step, Step::Progress { .. }),
    }
}

#[must_use]
pub fn step_label(lang: Lang, phase: Phase) -> &'static str {
    match phase {
        Phase::Download => lang.pick("Download", "Téléchargement"),
        Phase::Source => lang.pick("Checking the image", "Contrôle de l'image"),
        Phase::Authorize => lang.pick(
            "Waiting for the system's authorization",
            "En attente de l'autorisation du système",
        ),
        Phase::Write => lang.pick("Writing", "Écriture"),
        Phase::Flush => lang.pick("Flushing buffers", "Vidage des tampons"),
        Phase::Verify => lang.pick("Verifying", "Vérification"),
    }
}

/// Notification areas show a few lines; the rest stays in the window and the log.
fn first_line(message: &str) -> String {
    let line = message.lines().next().unwrap_or_default();
    match line.char_indices().nth(160) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_progression_donne_l_etape_et_le_pourcentage() {
        let step = Step::Progress {
            phase: Phase::Write,
            done: 512,
            total: Some(2048),
        };
        let notice = notice(&step, Lang::Fr);
        assert_eq!(notice.title, "Écriture");
        assert_eq!(notice.percent, Some(25));
        assert!(notice.body.starts_with("25 % · "), "{}", notice.body);
        assert!(!notice.last);
    }

    #[test]
    fn sans_taille_connue_seuls_les_octets_s_affichent() {
        let step = Step::Progress {
            phase: Phase::Download,
            done: 0,
            total: None,
        };
        let notice = notice(&step, Lang::En);
        assert_eq!((notice.title.as_str(), notice.percent), ("Download", None));
    }

    #[test]
    fn fin_echec_et_annulation_sont_les_dernieres() {
        for step in [
            Step::Written { verified: true },
            Step::Downloaded {
                path: "Download/a.iso".to_owned(),
            },
            Step::Failed {
                message: "E3005\nsuite".to_owned(),
            },
            Step::Cancelled,
        ] {
            assert!(notice(&step, Lang::Fr).last, "{step:?}");
        }
        let failed = notice(
            &Step::Failed {
                message: "E3005\nsuite".to_owned(),
            },
            Lang::Fr,
        );
        assert_eq!(failed.body, "E3005");
    }

    #[test]
    fn un_message_long_est_coupe() {
        let failed = notice(
            &Step::Failed {
                message: "é".repeat(400),
            },
            Lang::En,
        );
        assert_eq!(failed.body.chars().count(), 161);
    }
}
