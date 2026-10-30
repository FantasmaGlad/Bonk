//! Écriture d'une image depuis la fenêtre, par la ligne de commande.
//!
//! La fenêtre ne contient aucune logique d'écriture : elle lance `bonk write ... --progress json` dans
//! un **processus séparé** et traduit ses lignes JSON en barre de progression (CDC §9.1 : la GUI n'est
//! qu'une présentation des mêmes commandes). Conséquence voulue : l'interface ne tourne jamais avec
//! des droits élevés (CDC §10.2) ; seul ce processus a besoin des droits d'écriture, et tous les
//! garde-fous (filtre, confirmation, identité, démontage, vérification) sont ceux de la CLI.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};

use bonk_hal::INTERNAL_CONFIRMATION;
use iced::futures::channel::mpsc::{UnboundedReceiver, unbounded};
use serde_json::Value;

use crate::command::ViewOptions;

/// Ce que la fenêtre apprend du processus d'écriture.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Event {
    /// Le processus est lancé (numéro de processus, pour l'annulation).
    Started(u32),
    Progress {
        phase: String,
        done: u64,
        total: Option<u64>,
        rate: f64,
    },
    Finished(Outcome),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outcome {
    Done {
        bytes: u64,
        verified: bool,
        gpt_repaired: bool,
    },
    Failed {
        /// Code d'erreur stable (`bonk explain`), quand le processus en a donné un.
        code: Option<String>,
        message: String,
    },
}

/// Arguments de `bonk write` équivalents à ce que montre la fenêtre.
pub(crate) fn write_args(
    disk_id: &str,
    image: &str,
    options: ViewOptions,
    internal_confirmed: bool,
) -> Vec<String> {
    let mut args: Vec<String> = ["write", disk_id, image, "--yes", "--progress", "json"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    if options.show_internal {
        args.push("--show-internal".to_owned());
    }
    if options.allow_virtual {
        args.push("--allow-virtual".to_owned());
    }
    // La personne a confirmé dans la fenêtre : le sous-processus n'a pas de terminal pour le demander.
    if internal_confirmed {
        args.extend([
            "--confirm-internal".to_owned(),
            INTERNAL_CONFIRMATION.to_owned(),
        ]);
    }
    args
}

/// Traduit une ligne de la sortie standard de `bonk write --progress json`.
pub(crate) fn parse_line(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value["event"].as_str()? {
        "progress" => Some(Event::Progress {
            phase: value["phase"].as_str()?.to_owned(),
            done: value["bytes_done"].as_u64()?,
            total: value["bytes_total"].as_u64(),
            rate: value["bytes_per_second"].as_f64().unwrap_or(0.0),
        }),
        "result" => Some(Event::Finished(Outcome::Done {
            bytes: value["write"]["bytes"].as_u64()?,
            verified: !value["verify"].is_null(),
            gpt_repaired: !value["gpt_repair"].is_null(),
        })),
        _ => None,
    }
}

/// Échec d'après la sortie d'erreur : la dernière ligne `bonk : erreur : message [Exxxx]`.
pub(crate) fn parse_failure(stderr: &str, status: &str) -> Outcome {
    let line = stderr
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("bonk : erreur : "));
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

/// Lance `program args...` et rend le flux de ses événements. Les lectures se font dans des fils : la
/// fenêtre ne bloque jamais.
pub(crate) fn spawn(program: &Path, args: &[String]) -> UnboundedReceiver<Event> {
    let (sender, receiver) = unbounded();
    let (program, args) = (program.to_path_buf(), args.to_vec());
    std::thread::spawn(move || {
        let failed = |message: String| {
            let _ = sender.unbounded_send(Event::Finished(Outcome::Failed {
                code: None,
                message,
            }));
        };
        let mut child = match Command::new(&program)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return failed(format!(
                    "{} : lancement impossible : {e}",
                    program.display()
                ));
            }
        };
        let _ = sender.unbounded_send(Event::Started(child.id()));
        // La sortie d'erreur est lue à part : un processus bavard ne doit pas se bloquer dessus.
        let stderr_reader = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                text
            })
        });
        let mut result = None;
        if let Some(stdout) = child.stdout.take() {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                match parse_line(&line) {
                    Some(Event::Finished(outcome)) => result = Some(outcome),
                    Some(event) => {
                        let _ = sender.unbounded_send(event);
                    }
                    None => {}
                }
            }
        }
        let status = child.wait();
        let stderr = stderr_reader
            .and_then(|t| t.join().ok())
            .unwrap_or_default();
        let outcome = match (&status, result) {
            (Ok(s), Some(done)) if s.success() => done,
            (Ok(s), _) => parse_failure(&stderr, &s.to_string()),
            (Err(e), _) => parse_failure(&stderr, &e.to_string()),
        };
        let _ = sender.unbounded_send(Event::Finished(outcome));
    });
    receiver
}

/// Demande l'arrêt propre du processus d'écriture (il s'arrête entre deux blocs).
pub(crate) fn cancel(pid: u32) {
    if let Err(error) = bonk_hal::interrupt::stop_process(pid) {
        log::warn!(target: "bonk::gui", pid = pid, error = error.to_string().as_str(); "write process not stopped");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    // Lu seulement par les tests qui lancent un vrai processus (Unix).
    #[cfg(unix)]
    use iced::futures::StreamExt;

    use super::*;

    #[test]
    fn arguments_de_la_commande() {
        assert_eq!(
            write_args("usb-a", "/tmp/x.img", ViewOptions::default(), false),
            [
                "write",
                "usb-a",
                "/tmp/x.img",
                "--yes",
                "--progress",
                "json"
            ]
        );
        let options = ViewOptions {
            show_internal: true,
            allow_virtual: true,
            show_all: true,
        };
        let args = write_args("usb-a", "x", options, true);
        assert!(
            args.contains(&"--show-internal".to_owned())
                && args.contains(&"--allow-virtual".to_owned())
        );
        assert!(args.ends_with(&["--confirm-internal".to_owned(), "confirme".to_owned()]));
        // `--all` n'a pas de sens pour une écriture.
        assert!(!args.contains(&"--all".to_owned()));
    }

    #[test]
    fn lignes_de_progression_et_de_resultat() {
        let progress = r#"{"event":"progress","schema_version":1,"phase":"write","bytes_done":1048576,"bytes_total":4194304,"bytes_per_second":1e7,"elapsed_ms":100}"#;
        assert_eq!(
            parse_line(progress),
            Some(Event::Progress {
                phase: "write".into(),
                done: 1_048_576,
                total: Some(4_194_304),
                rate: 1e7
            })
        );
        let unknown_total = r#"{"event":"progress","phase":"verify","bytes_done":5,"bytes_total":null,"bytes_per_second":0.0}"#;
        assert!(matches!(
            parse_line(unknown_total),
            Some(Event::Progress { total: None, .. })
        ));
        let result = r#"{"event":"result","write":{"bytes":4194304},"verify":{"bytes":4194304},"gpt_repair":null}"#;
        assert_eq!(
            parse_line(result),
            Some(Event::Finished(Outcome::Done {
                bytes: 4_194_304,
                verified: true,
                gpt_repaired: false
            }))
        );
        assert_eq!(parse_line("pas du json"), None);
        assert_eq!(parse_line(r#"{"event":"plan"}"#), None);
        assert_eq!(parse_line(r#"{"event":"progress"}"#), None);
    }

    #[test]
    fn echec_d_apres_la_sortie_d_erreur() {
        let stderr = "2026-10-29T10:00:00.000Z WARN cli quelque chose\nbonk : erreur : disque introuvable : « x » [E1003]\n";
        assert_eq!(
            parse_failure(stderr, "exit status: 1"),
            Outcome::Failed {
                code: Some("E1003".into()),
                message: "disque introuvable : « x »".into()
            }
        );
        assert_eq!(
            parse_failure("bonk : erreur : sans code\n", "exit status: 1"),
            Outcome::Failed {
                code: None,
                message: "sans code".into()
            }
        );
        let silent = parse_failure("", "signal: 9");
        assert!(
            matches!(&silent, Outcome::Failed { code: None, message } if message.contains("signal: 9"))
        );
    }

    #[cfg(unix)]
    async fn collect(script: &str) -> Vec<Event> {
        spawn(Path::new("/bin/sh"), &["-c".to_owned(), script.to_owned()])
            .collect()
            .await
    }

    #[cfg(unix)]
    #[test]
    fn un_processus_qui_reussit() {
        let script = r#"echo '{"event":"progress","phase":"write","bytes_done":10,"bytes_total":20,"bytes_per_second":1.0}'; echo '{"event":"result","write":{"bytes":20},"verify":{"bytes":20},"gpt_repair":{"actions":[]}}'"#;
        let events = iced::futures::executor::block_on(collect(script));
        assert!(matches!(events[0], Event::Started(_)));
        assert!(matches!(events[1], Event::Progress { done: 10, .. }));
        assert_eq!(
            events.last(),
            Some(&Event::Finished(Outcome::Done {
                bytes: 20,
                verified: true,
                gpt_repaired: true
            }))
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Finished(_)))
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn un_processus_qui_echoue_avec_son_code() {
        let events = iced::futures::executor::block_on(collect(
            "echo 'bonk : erreur : cible refusée [E2001]' >&2; exit 3",
        ));
        assert_eq!(
            events.last(),
            Some(&Event::Finished(Outcome::Failed {
                code: Some("E2001".into()),
                message: "cible refusée".into()
            }))
        );
        // Un résultat affiché mais un code de sortie non nul reste un échec.
        let events = iced::futures::executor::block_on(collect(
            r#"echo '{"event":"result","write":{"bytes":1}}'; echo 'bonk : erreur : après coup [E4001]' >&2; exit 1"#,
        ));
        assert!(matches!(
            events.last(),
            Some(Event::Finished(Outcome::Failed { .. }))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn programme_introuvable_et_annulation() {
        let events = iced::futures::executor::block_on(async {
            spawn(Path::new("/n/existe/pas"), &[])
                .collect::<Vec<_>>()
                .await
        });
        assert!(
            matches!(events.as_slice(), [Event::Finished(Outcome::Failed { message, .. })] if message.contains("lancement impossible"))
        );
        // L'annulation envoie SIGTERM au processus.
        let mut stream = spawn(
            Path::new("/bin/sh"),
            &["-c".to_owned(), "sleep 30".to_owned()],
        );
        let pid = iced::futures::executor::block_on(async {
            match stream.next().await {
                Some(Event::Started(pid)) => Some(pid),
                _ => None,
            }
        })
        .expect("le processus démarre");
        cancel(pid);
        let end = iced::futures::executor::block_on(stream.collect::<Vec<_>>());
        assert!(matches!(
            end.last(),
            Some(Event::Finished(Outcome::Failed { .. }))
        ));
    }
}
