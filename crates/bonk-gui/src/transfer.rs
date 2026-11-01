//! Écriture d'une image depuis la fenêtre, par la ligne de commande.
//!
//! La fenêtre ne contient aucune logique d'écriture : elle lance `bonk write ... --progress json` dans
//! un **processus séparé** et traduit ses lignes JSON en barre de progression. Conséquence voulue :
//! l'interface ne tourne jamais avec des droits élevés ; seul ce processus a besoin des droits
//! d'écriture, et tous les garde-fous (filtre, confirmation, identité, démontage, vérification) sont
//! ceux de la CLI.

use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};

#[cfg(test)]
use bonk_ui::event::write_args;
pub(crate) use bonk_ui::event::{Event, Outcome, parse_failure, parse_line};
use iced::futures::channel::mpsc::{UnboundedReceiver, unbounded};

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
        let mut command = Command::new(&program);
        bonk_hal::console::hide_child_console(&mut command);
        let mut child = match command
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                return failed(format!("{}: cannot start: {e}", program.display()));
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

    use bonk_core::write::Phase;
    use bonk_ui::command::{ViewOptions, WriteView};

    use super::*;

    #[test]
    fn arguments_de_la_commande() {
        assert_eq!(
            write_args(
                "usb-a",
                WriteView::image("/tmp/x.img"),
                ViewOptions::default(),
                false,
                None
            ),
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
        let args = write_args(
            "usb-a",
            WriteView::image("x"),
            options,
            true,
            Some("socks5h://t"),
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--proxy", "socks5h://t"])
        );
        assert!(
            args.contains(&"--show-internal".to_owned())
                && args.contains(&"--allow-virtual".to_owned())
        );
        assert!(args.contains(&"--force".to_owned()));
        // `--all` n'a pas de sens pour une écriture.
        assert!(!args.contains(&"--all".to_owned()));
    }

    #[test]
    fn lignes_de_progression_et_de_resultat() {
        let progress = r#"{"event":"progress","schema_version":1,"phase":"write","bytes_done":1048576,"bytes_total":4194304,"bytes_per_second":1e7,"elapsed_ms":100}"#;
        assert_eq!(
            parse_line(progress),
            Some(Event::Progress {
                phase: Phase::Write,
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
                gpt_repaired: false,
                secure_boot: None,
            }))
        );
        assert_eq!(parse_line("pas du json"), None);
        assert_eq!(parse_line(r#"{"event":"plan"}"#), None);
        assert_eq!(parse_line(r#"{"event":"progress"}"#), None);
    }

    #[test]
    fn echec_d_apres_la_sortie_d_erreur() {
        let stderr = "2026-10-29T10:00:00.000Z WARN cli quelque chose\nbonk: error: disque introuvable : « x » [E1003]\n";
        assert_eq!(
            parse_failure(stderr, "exit status: 1"),
            Outcome::Failed {
                code: Some("E1003".into()),
                message: "disque introuvable : « x »".into()
            }
        );
        assert_eq!(
            parse_failure("bonk: error: sans code\n", "exit status: 1"),
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

    #[test]
    fn un_evenement_d_erreur_donne_son_code_et_une_phase_inconnue_est_ignoree() {
        let line = r#"{"event":"error","code":"E3005","message":"empreinte fausse"}"#;
        assert_eq!(
            parse_line(line),
            Some(Event::Finished(Outcome::Failed {
                code: Some("E3005".into()),
                message: "empreinte fausse".into(),
            }))
        );
        let download = r#"{"event":"progress","phase":"download","bytes_done":1,"bytes_total":2,"bytes_per_second":1.0}"#;
        assert!(matches!(
            parse_line(download),
            Some(Event::Progress {
                phase: Phase::Download,
                ..
            })
        ));
        let unknown = r#"{"event":"progress","phase":"plus_tard","bytes_done":1,"bytes_total":2}"#;
        assert_eq!(parse_line(unknown), None);
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
                gpt_repaired: true,
                secure_boot: None,
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
            "echo 'bonk: error: cible refusée [E2001]' >&2; exit 3",
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
            r#"echo '{"event":"result","write":{"bytes":1}}'; echo 'bonk: error: après coup [E4001]' >&2; exit 1"#,
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
            matches!(events.as_slice(), [Event::Finished(Outcome::Failed { message, .. })] if message.contains("cannot start impossible"))
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
