use std::time::Duration;

use bonk_core::write::Phase;
use bonk_hal::{Lang, format_size};

use crate::errors::failure_text;
use crate::event::Outcome;
use crate::model::WRITE_DELAY;

/// Seconds shown in the countdown, from the time since opening.
#[must_use]
pub fn secondes_restantes(elapsed: Duration) -> u64 {
    let reste = WRITE_DELAY.saturating_sub(elapsed);
    reste.as_secs() + u64::from(reste.subsec_nanos() > 0)
}

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

#[must_use]
pub fn outcome(lang: Lang, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Done {
            bytes,
            verified,
            gpt_repaired,
            secure_boot,
            bios_boot,
        } => format!(
            "{} {} {}{}{}{}.{}",
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
            if *bios_boot {
                lang.pick(", boots in BIOS and UEFI", ", démarre en BIOS et en UEFI")
            } else {
                ""
            },
            secure_boot
                .as_ref()
                .map_or_else(String::new, |signer| secure_boot_text(lang, signer)),
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

fn secure_boot_text(lang: Lang, signer: &bonk_core::pe::Signer) -> String {
    use bonk_core::pe::{PCA_2011, Signer, UEFI_CA_2023};
    let sentence = match (signer, lang) {
        (Signer::WindowsProductionPca2011, Lang::En) => format!(
            "loader signed by {PCA_2011}: with Secure Boot, refused by a PC that revoked this certificate"
        ),
        (Signer::WindowsProductionPca2011, Lang::Fr) => format!(
            "chargeur signé par {PCA_2011} : avec Secure Boot, refusé par un PC qui a révoqué ce certificat"
        ),
        (Signer::WindowsUefiCa2023, Lang::En) => format!(
            "loader signed by {UEFI_CA_2023}: with Secure Boot, refused by a PC whose firmware does not know this certificate"
        ),
        (Signer::WindowsUefiCa2023, Lang::Fr) => format!(
            "chargeur signé par {UEFI_CA_2023} : avec Secure Boot, refusé par un PC dont le micrologiciel ne connaît pas ce certificat"
        ),
        (Signer::Other(name), Lang::En) => {
            format!("loader signed by {name}: with Secure Boot, depends on what the PC accepts")
        }
        (Signer::Other(name), Lang::Fr) => {
            format!("chargeur signé par {name} : avec Secure Boot, dépend de ce que le PC accepte")
        }
        (Signer::Unsigned, Lang::En) => {
            "unsigned loader: refused with Secure Boot active".to_owned()
        }
        (Signer::Unsigned, Lang::Fr) => {
            "chargeur non signé : refusé quand Secure Boot est actif".to_owned()
        }
    };
    match lang {
        Lang::En => format!(" Secure Boot: {sentence}."),
        Lang::Fr => format!(" Secure Boot : {sentence}."),
    }
}

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

#[cfg(test)]
mod tests {
    use bonk_core::pe::Signer;

    use super::*;

    fn windows(signer: Signer) -> Outcome {
        Outcome::Done {
            bytes: 7 << 30,
            verified: true,
            gpt_repaired: false,
            secure_boot: Some(signer),
            bios_boot: false,
        }
    }

    /// The Secure Boot sentence follows the window language with its typography: no space before the colon in
    /// English.
    #[test]
    fn la_phrase_secure_boot_suit_la_langue() {
        let fr = outcome(Lang::Fr, &windows(Signer::WindowsProductionPca2011));
        assert!(
            fr.contains("Secure Boot : chargeur signé par Microsoft Windows Production PCA 2011"),
            "{fr}"
        );
        let en = outcome(Lang::En, &windows(Signer::WindowsProductionPca2011));
        assert!(
            en.contains("Secure Boot: loader signed by Microsoft Windows Production PCA 2011"),
            "{en}"
        );
        assert!(!en.contains(" :"), "{en}");
        let unsigned = outcome(Lang::Fr, &windows(Signer::Unsigned));
        assert!(unsigned.contains("chargeur non signé"), "{unsigned}");
    }

    #[test]
    fn une_cle_qui_demarre_aussi_en_bios_le_dit() {
        let line = serde_json::json!({
            "event": "result",
            "write": {"bytes": 1},
            "verify": {"bytes": 1},
            "gpt_repair": null,
            "extraction": {"signer": "unsigned", "firmware": ["uefi", "bios"]},
        })
        .to_string();
        let Some(crate::event::Event::Finished(done)) = crate::event::parse_line(&line) else {
            panic!("résultat non lu");
        };
        assert!(matches!(
            done,
            Outcome::Done {
                bios_boot: true,
                ..
            }
        ));
        assert!(outcome(Lang::Fr, &done).contains("démarre en BIOS et en UEFI"));
        assert!(!outcome(Lang::En, &windows(Signer::Unsigned)).contains("BIOS"));
    }

    /// A write process stopped without an error line: the title follows the window language, never French in
    /// an English window.
    #[test]
    fn un_arret_sans_message_suit_la_langue() {
        let failure = crate::event::parse_failure("", "signal 9");
        let en = outcome(Lang::En, &failure);
        assert!(!en.contains("processus"), "{en}");
        assert!(en.contains("[E4001]"), "{en}");
        let fr = outcome(Lang::Fr, &failure);
        assert!(fr.contains("[E4001]"), "{fr}");
    }

    #[test]
    fn le_signataire_se_lit_dans_le_resultat_de_bonk_write() {
        for signer in [
            Signer::WindowsUefiCa2023,
            Signer::Other("Debian".to_owned()),
            Signer::Unsigned,
        ] {
            let line = serde_json::json!({
                "event": "result",
                "write": {"bytes": 1},
                "verify": {"bytes": 1},
                "gpt_repair": null,
                "extraction": {"signer": signer, "secure_boot": "anglais"},
            })
            .to_string();
            let parsed = crate::event::parse_line(&line);
            assert!(
                matches!(parsed, Some(crate::event::Event::Finished(Outcome::Done { secure_boot: Some(ref s), .. })) if *s == signer),
                "{line}"
            );
        }
    }
}
