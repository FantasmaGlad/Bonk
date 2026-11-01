//! Les phrases que toutes les interfaces affichent : une seule rédaction, en deux langues.

use std::time::Duration;

use bonk_core::write::Phase;
use bonk_hal::{Lang, format_size};

use crate::errors::failure_text;
use crate::event::Outcome;
use crate::model::WRITE_DELAY;

/// Secondes affichées dans le décompte du délai de réflexion, selon le temps écoulé depuis l'ouverture.
#[must_use]
pub fn secondes_restantes(elapsed: Duration) -> u64 {
    let reste = WRITE_DELAY.saturating_sub(elapsed);
    reste.as_secs() + u64::from(reste.subsec_nanos() > 0)
}

/// Le libellé du bouton d'écriture, selon le délai et l'image.
#[must_use]
pub fn write_button(lang: Lang, elapsed: Duration, image_empty: bool) -> String {
    if elapsed < WRITE_DELAY {
        let remaining = secondes_restantes(elapsed);
        match lang {
            Lang::En => format!("Write and erase this disk (in {remaining} s)"),
            Lang::Fr => format!("Écrire et effacer ce disque (dans {remaining} s)"),
        }
    } else if image_empty {
        lang.pick(
            "Write and erase this disk (choose an image)",
            "Écrire et effacer ce disque (choisir une image)",
        )
        .to_owned()
    } else {
        lang.pick("Write and erase this disk", "Écrire et effacer ce disque")
            .to_owned()
    }
}

#[must_use]
pub fn step(lang: Lang, phase: Phase) -> &'static str {
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

/// La ligne de progression et la part accomplie, de 0 à 1.
#[must_use]
pub fn progress(
    lang: Lang,
    phase: Phase,
    done: u64,
    total: Option<u64>,
    rate: f64,
) -> (String, f32) {
    let fraction = total
        .filter(|t| *t > 0)
        .map_or(0.0, |t| (done as f32 / t as f32).clamp(0.0, 1.0));
    let line = format!(
        "{}{} {}{}  ({}/s)",
        step(lang, phase),
        lang.pick(":", " :"),
        format_size(done, lang),
        total.map_or_else(String::new, |t| format!(
            " {} {}",
            lang.pick("of", "sur"),
            format_size(t, lang)
        )),
        format_size(rate as u64, lang),
    );
    (line, fraction)
}

/// Le résultat d'une écriture, en toutes lettres.
#[must_use]
pub fn outcome(lang: Lang, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Done {
            bytes,
            verified,
            gpt_repaired,
            secure_boot,
        } => format!(
            "{} {} {}{}{}.{}",
            lang.pick("WRITE FINISHED:", "ÉCRITURE TERMINÉE :"),
            format_size(*bytes, lang),
            lang.pick("written", "écrits"),
            if *verified {
                lang.pick(", read back and verified", ", relus et vérifiés")
            } else {
                ""
            },
            if *gpt_repaired {
                lang.pick(", backup GPT repaired", ", GPT de secours réparé")
            } else {
                ""
            },
            secure_boot
                .as_ref()
                .map_or_else(String::new, |text| format!(" Secure Boot : {text}.")),
        ),
        Outcome::Failed { code, message } => format!(
            "{} {}{}",
            lang.pick("FAILED:", "ÉCHEC :"),
            failure_text(lang, code.as_deref(), message),
            code.as_ref().map_or_else(String::new, |c| match lang {
                Lang::En => format!(" [{c}]: see \"bonk explain {c}\""),
                Lang::Fr => format!(" [{c}] : voir « bonk explain {c} »"),
            }),
        ),
    }
}

/// Le récapitulatif d'un disque avant de l'effacer.
#[must_use]
pub fn disk_summary(lang: Lang, disk: &bonk_hal::Disk) -> String {
    let labels: Vec<&str> = disk
        .partitions
        .iter()
        .filter_map(|p| p.label.as_deref())
        .collect();
    format!(
        "{}  {} {}  {}  {} partition(s){}",
        disk.model,
        lang.pick("serial", "série"),
        disk.serial
            .as_deref()
            .unwrap_or(lang.pick("unknown", "inconnue")),
        format_size(disk.size_bytes, lang),
        disk.partitions.len(),
        if labels.is_empty() {
            String::new()
        } else {
            format!(
                "  {} {}",
                lang.pick("labels:", "étiquettes :"),
                labels.join(", ")
            )
        },
    )
}
