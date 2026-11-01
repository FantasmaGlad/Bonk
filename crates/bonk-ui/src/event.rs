//! Ce que rapporte une écriture, et la commande qui la lance sur un ordinateur.

use serde_json::Value;

use bonk_core::write::Phase;

use crate::command::{ViewOptions, WriteView};

/// Ce que la fenêtre apprend du processus d'écriture.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Le processus est lancé (numéro de processus, pour l'annulation).
    Started(u32),
    Progress {
        phase: Phase,
        done: u64,
        total: Option<u64>,
        rate: f64,
    },
    Finished(Outcome),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done {
        bytes: u64,
        verified: bool,
        gpt_repaired: bool,
        /// Clé Windows : ce que le certificat du chargeur implique.
        secure_boot: Option<String>,
    },
    Failed {
        /// Code d'erreur stable (`bonk explain`), quand le processus en a donné un.
        code: Option<String>,
        message: String,
    },
}

/// Arguments de `bonk write` équivalents à ce que montre la fenêtre.
pub fn write_args(
    disk_id: &str,
    write: WriteView<'_>,
    options: ViewOptions,
    internal_confirmed: bool,
    proxy: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = ["write", disk_id, write.image, "--yes", "--progress", "json"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    args.extend(write.flags());
    if let Some(proxy) = proxy {
        args.extend(["--proxy".to_owned(), proxy.to_owned()]);
    }
    if options.show_internal {
        args.push("--show-internal".to_owned());
    }
    if options.allow_virtual {
        args.push("--allow-virtual".to_owned());
    }
    // La personne a confirmé dans la fenêtre : `bonk write` exige alors `--force`.
    if internal_confirmed {
        args.push("--force".to_owned());
    }
    args
}

/// Traduit une ligne de la sortie standard de `bonk write --progress json`.
pub fn parse_line(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value["event"].as_str()? {
        "progress" => Some(Event::Progress {
            phase: serde_json::from_value(value["phase"].clone()).ok()?,
            done: value["bytes_done"].as_u64()?,
            total: value["bytes_total"].as_u64(),
            rate: value["bytes_per_second"].as_f64().unwrap_or(0.0),
        }),
        "result" => Some(Event::Finished(Outcome::Done {
            bytes: value["write"]["bytes"].as_u64()?,
            verified: !value["verify"].is_null(),
            gpt_repaired: !value["gpt_repair"].is_null(),
            secure_boot: value["extraction"]["secure_boot"]
                .as_str()
                .map(str::to_owned),
        })),
        "error" => Some(Event::Finished(Outcome::Failed {
            code: value["code"].as_str().map(str::to_owned),
            message: value["message"].as_str()?.to_owned(),
        })),
        _ => None,
    }
}

/// Échec d'un processus qui n'a pas pu donner son événement `error` (arguments refusés, arrêt brutal),
/// d'après la sortie d'erreur : la dernière ligne `bonk: error: message [Exxxx]`.
pub fn parse_failure(stderr: &str, status: &str) -> Outcome {
    let line = stderr
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("bonk: error: "));
    match line {
        Some(text) => {
            let (message, code) = match text.rsplit_once(" [E") {
                Some((message, rest)) if rest.ends_with(']') && rest.len() == 5 => {
                    (message, Some(format!("E{}", &rest[..4])))
                }
                _ => (text, None),
            };
            Outcome::Failed {
                code,
                message: message.to_owned(),
            }
        }
        None => Outcome::Failed {
            code: None,
            message: format!("le processus d'écriture s'est arrêté ({status})"),
        },
    }
}
