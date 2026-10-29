//! Ligne de commande de Bonk : analyse des arguments, sortie texte et JSON, journal, codes de sortie
//! stables (CDC §9).
//!
//! Ce crate ne connaît pas la fenêtre : le binaire lui passe la fonction qui l'ouvre. Ainsi `bonk` sans
//! commande ouvre la fenêtre, et `bonk -v` l'ouvre avec le journal verbeux.

mod debug;
mod diagnose;
mod info;
mod list;
mod logging;

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use bonk_hal::{Disk, EnumerateError, enumerate_disks};
use clap::{Parser, Subcommand};

use crate::logging::{LogArgs, Settings};

/// Code de sortie d'une erreur d'exécution (énumération impossible, écriture de la sortie en échec,
/// fenêtre impossible à ouvrir). Les erreurs d'usage gardent le code `2` de clap.
const EXIT_RUNTIME_ERROR: u8 = 1;
/// Code de sortie d'une erreur d'usage, celui de clap.
const EXIT_USAGE: u8 = 2;

/// Arguments de la ligne de commande de Bonk.
///
/// Sans commande, la fenêtre s'ouvre.
#[derive(Debug, Parser)]
// `long_about = None` : sans lui, clap reprend ce commentaire de documentation comme texte long
// et `--help` afficherait une note de développeur au lieu de la description de l'outil.
#[command(
    name = "bonk",
    version,
    about = "Création de supports d'amorçage (clés USB et disques externes)",
    long_about = None,
)]
struct Cli {
    #[command(flatten)]
    log: LogArgs,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Disques : liste et détail
    Disk {
        #[command(subcommand)]
        action: DiskAction,
    },
    /// Liste les disques que Bonk peut écrire (raccourci de `disk list`)
    List(list::ListArgs),
    /// Détail d'un disque (raccourci de `disk info`)
    Info(info::InfoArgs),
    /// Rapport de support en JSON : système, rendu, disques et verdicts, secrets masqués
    Diagnose(diagnose::DiagnoseArgs),
    /// Bas niveau des disques, en lecture seule : sources système, filtre pas à pas, tables de partitions
    Debug {
        #[command(subcommand)]
        action: debug::DebugAction,
    },
    /// Ouvre la fenêtre (comme `bonk` seul)
    Gui,
}

#[derive(Debug, Subcommand)]
enum DiskAction {
    /// Liste les disques que Bonk peut écrire (lecture seule)
    List(list::ListArgs),
    /// Détail d'un disque (lecture seule)
    Info(info::InfoArgs),
}

/// Vérifie qu'une ligne de commande est acceptée par la CLI, sans l'exécuter, et rend ses mots. Sert
/// au test de parité (CDC §9.1) : chaque commande équivalente que la fenêtre affiche doit être valide.
///
/// # Errors
/// Guillemets mal fermés, ou ligne que la CLI refuse (le message de clap est repris).
pub fn check_command_line(command: &str) -> Result<Vec<String>, String> {
    let words = split_words(command)?;
    Cli::try_parse_from(&words).map_err(|erreur| erreur.to_string())?;
    Ok(words)
}

/// Découpe une ligne en mots à la façon d'un shell POSIX : espaces, apostrophes simples, guillemets
/// doubles. Les variables et substitutions ne sont pas interprétées (la fenêtre n'en produit pas).
fn split_words(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => current.push(c),
            // Hors guillemets, l'antislash rend le caractère suivant littéral (`'\''` dans le texte
            // d'un identifiant contenant une apostrophe).
            (None, '\\') => {
                current.push(chars.next().ok_or("antislash final")?);
                in_word = true;
            }
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            (None, c) => {
                current.push(c);
                in_word = true;
            }
        }
    }
    if quote.is_some() {
        return Err("guillemet non fermé".to_owned());
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

/// Exécute la ligne de commande et renvoie le code de sortie du processus. `gui` ouvre la fenêtre et
/// rend la main à sa fermeture.
///
/// Codes de sortie, stables d'une version à l'autre :
/// - `0` : succès, `--help` et `--version` compris, même si aucun disque n'est proposé ;
/// - `1` : erreur d'exécution (énumération des disques impossible ou non disponible sur ce système,
///   fenêtre impossible à ouvrir) ;
/// - `2` : erreur d'usage (code de clap), réglage de journal invalide compris.
pub fn run<I, T, G>(args: I, gui: G) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    G: FnOnce() -> Result<(), String>,
{
    // Flux non verrouillés : `execute` ouvre la fenêtre, pendant laquelle d'autres fils journalisent sur
    // la sortie d'erreur. Un verrou tenu pendant toute l'exécution les bloquerait pour toujours
    // (l'énumération des disques de la fenêtre incluse).
    ExitCode::from(execute(
        args,
        std::env::var("BONK_LOG").ok().as_deref(),
        &mut io::stdout(),
        &mut io::stderr(),
        logging::install,
        enumerate_disks,
        gui,
    ))
}

/// Cœur de [`run`], avec les flux, l'environnement et les dépendances injectés pour les tests.
fn execute<I, T, O, E, L, F, G>(
    args: I,
    env_log: Option<&str>,
    out: &mut O,
    err: &mut E,
    install_log: L,
    enumerate: F,
    gui: G,
) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    O: Write,
    E: Write,
    L: FnOnce(Settings) -> Result<(), String>,
    F: FnOnce() -> Result<Vec<Disk>, EnumerateError>,
    G: FnOnce() -> Result<(), String>,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(erreur) => {
            // `print` choisit la sortie adaptée : aide et version sur stdout, erreurs sur stderr.
            // Un échec d'écriture sur ces flux ne doit pas masquer le code de sortie.
            let _ = erreur.print();
            return u8::try_from(erreur.exit_code()).unwrap_or(EXIT_USAGE);
        }
    };

    let settings = match Settings::from_args(&cli.log, env_log) {
        Ok(settings) => settings,
        Err(erreur) => {
            let _ = writeln!(err, "bonk : erreur : {erreur}");
            return EXIT_USAGE;
        }
    };
    if let Err(erreur) = install_log(settings) {
        let _ = writeln!(err, "bonk : erreur : {erreur}");
        return EXIT_RUNTIME_ERROR;
    }
    log::info!(
        target: "bonk::cli",
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH;
        "start"
    );

    match cli.command {
        None | Some(Command::Gui) => {
            log::debug!(target: "bonk::cli", "opening the window");
            match gui() {
                Ok(()) => 0,
                Err(erreur) => {
                    // Pas d'affichage (SSH, serveur) ou pas de rendu utilisable : la CLI reste là.
                    log::debug!(target: "bonk::cli", reason = erreur.as_str(); "window failed to open");
                    let _ = writeln!(
                        err,
                        "bonk : erreur : impossible d'ouvrir l'interface graphique : {erreur}\n\
                         Utilisez la ligne de commande : bonk --help"
                    );
                    EXIT_RUNTIME_ERROR
                }
            }
        }
        Some(Command::Disk { action }) => match action {
            DiskAction::List(args) => run_list(&args, out, err, enumerate),
            DiskAction::Info(args) => run_info(&args, out, err, enumerate),
        },
        Some(Command::List(args)) => run_list(&args, out, err, enumerate),
        Some(Command::Info(args)) => run_info(&args, out, err, enumerate),
        Some(Command::Debug { action }) => run_debug(action, out, err, enumerate),
        Some(Command::Diagnose(args)) => {
            log::debug!(target: "bonk::cli", include_serials = args.include_serials(); "command diagnose");
            let result = enumerate();
            if let Err(erreur) = &result {
                log::debug!(target: "bonk::cli", reason = erreur.to_string().as_str(); "enumeration failed (reported in the report)");
            }
            match diagnose::run(&args, &diagnose::Environment::system(), &result, out) {
                Ok(()) => 0,
                Err(erreur) => {
                    let _ = writeln!(
                        err,
                        "bonk : erreur : rapport impossible à écrire : {erreur}"
                    );
                    EXIT_RUNTIME_ERROR
                }
            }
        }
    }
}

/// Énumère les disques ; en cas d'échec, écrit l'erreur et renvoie le code de sortie.
fn disks_or_exit<E, F>(err: &mut E, enumerate: F) -> Result<Vec<Disk>, u8>
where
    E: Write,
    F: FnOnce() -> Result<Vec<Disk>, EnumerateError>,
{
    enumerate().map_err(|erreur| {
        log::debug!(target: "bonk::cli", reason = erreur.to_string().as_str(); "enumeration failed");
        let _ = writeln!(err, "bonk : erreur : {erreur}");
        EXIT_RUNTIME_ERROR
    })
}

fn run_debug<O: Write, E: Write, F>(
    action: debug::DebugAction,
    out: &mut O,
    err: &mut E,
    enumerate: F,
) -> u8
where
    F: FnOnce() -> Result<Vec<Disk>, EnumerateError>,
{
    use debug::{DebugAction, Outcome};
    log::debug!(target: "bonk::cli", action = format!("{action:?}").as_str(); "command debug");
    let outcome = match action {
        DebugAction::Sources(args) => {
            let (result, reads) = bonk_hal::enumerate_disks_recorded();
            debug::sources(&args, &result, &reads, out)
        }
        DebugAction::Filter(args) => match disks_or_exit(err, enumerate) {
            Ok(disks) => debug::filter(&args, &disks, out),
            Err(code) => return code,
        },
        DebugAction::Layout(args) => debug::layout(&args, out),
        DebugAction::Hexdump(args) => debug::hexdump(&args, out),
    };
    match outcome {
        Ok(Outcome::Done) => 0,
        Ok(Outcome::Failed(message)) => {
            let _ = writeln!(err, "bonk : erreur : {message}");
            EXIT_RUNTIME_ERROR
        }
        Err(erreur) => {
            let _ = writeln!(err, "bonk : erreur d'écriture de la sortie : {erreur}");
            EXIT_RUNTIME_ERROR
        }
    }
}

fn run_list<O: Write, E: Write, F>(
    args: &list::ListArgs,
    out: &mut O,
    err: &mut E,
    enumerate: F,
) -> u8
where
    F: FnOnce() -> Result<Vec<Disk>, EnumerateError>,
{
    log::debug!(target: "bonk::cli", all = args.all(), json = args.json(); "command list");
    let disks = match disks_or_exit(err, enumerate) {
        Ok(disks) => disks,
        Err(code) => return code,
    };
    match list::run(args, &disks, out) {
        Ok(()) => 0,
        Err(erreur) => {
            let _ = writeln!(err, "bonk : erreur d'écriture de la sortie : {erreur}");
            EXIT_RUNTIME_ERROR
        }
    }
}

fn run_info<O: Write, E: Write, F>(
    args: &info::InfoArgs,
    out: &mut O,
    err: &mut E,
    enumerate: F,
) -> u8
where
    F: FnOnce() -> Result<Vec<Disk>, EnumerateError>,
{
    log::debug!(target: "bonk::cli", target_disk = args.target(); "command info");
    let disks = match disks_or_exit(err, enumerate) {
        Ok(disks) => disks,
        Err(code) => return code,
    };
    match info::run(args, &disks, out) {
        Ok(true) => 0,
        Ok(false) => {
            let _ = writeln!(
                err,
                "bonk : erreur : disque introuvable : « {} » (identifiants : bonk list --all)",
                args.target()
            );
            EXIT_RUNTIME_ERROR
        }
        Err(erreur) => {
            let _ = writeln!(err, "bonk : erreur d'écriture de la sortie : {erreur}");
            EXIT_RUNTIME_ERROR
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use bonk_hal::{Bus, DiskKind, Partition};
    use serde_json::Value;

    use super::*;

    fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
        Partition {
            path: path.to_owned(),
            label: None,
            fs_type: Some("ext4".to_owned()),
            size_bytes: 1 << 30,
            mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
            holders: vec![],
            is_system_volume: system,
            is_active_esp: false,
        }
    }

    fn disk(id: &str, path: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
        Disk {
            id: id.to_owned(),
            path: path.to_owned(),
            model: "Modèle de test".to_owned(),
            serial: None,
            size_bytes: 16_008_609_792,
            bus,
            kind: DiskKind::Physical,
            removable_flag: false,
            fs_type: None,
            holders: vec![],
            partitions,
        }
    }

    fn sample() -> Vec<Disk> {
        vec![
            disk(
                "nvme-systeme",
                "/dev/nvme0n1",
                Bus::Nvme,
                vec![partition("/dev/nvme0n1p2", &["/"], true)],
            ),
            disk("usb-cle", "/dev/sdb", Bus::Usb, vec![]),
            disk("sata-libre", "/dev/sda", Bus::Sata, vec![]),
        ]
    }

    /// Ce que `execute` a fait : code, sorties, et réglages de journal reçus.
    struct Resultat {
        code: u8,
        out: String,
        err: String,
        journal_installe: bool,
        fenetre_ouverte: bool,
    }

    fn lancer_avec(
        args: &[&str],
        env_log: Option<&str>,
        disks: Vec<Disk>,
        fenetre: Result<(), String>,
    ) -> Resultat {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let journal = RefCell::new(false);
        let ouverte = RefCell::new(false);
        let code = execute(
            std::iter::once("bonk").chain(args.iter().copied()),
            env_log,
            &mut out,
            &mut err,
            |_| {
                *journal.borrow_mut() = true;
                Ok(())
            },
            || Ok(disks),
            || {
                *ouverte.borrow_mut() = true;
                fenetre
            },
        );
        Resultat {
            code,
            out: String::from_utf8_lossy(&out).into_owned(),
            err: String::from_utf8_lossy(&err).into_owned(),
            journal_installe: *journal.borrow(),
            fenetre_ouverte: *ouverte.borrow(),
        }
    }

    /// Lance la commande avec des disques fixes ; renvoie code, sortie standard, sortie d'erreur.
    fn lancer(args: &[&str], disks: Vec<Disk>) -> (u8, String, String) {
        let r = lancer_avec(args, None, disks, Ok(()));
        (r.code, r.out, r.err)
    }

    #[test]
    fn liste_par_defaut_ne_montre_que_les_disques_proposes() {
        let (code, out, err) = lancer(&["list"], sample());
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(out.contains("usb-cle"), "{out}");
        assert!(out.contains("proposé"), "{out}");
        assert!(!out.contains("nvme-systeme"), "{out}");
        assert!(!out.contains("sata-libre"), "{out}");
    }

    #[test]
    fn all_montre_les_ecartes_avec_la_raison() {
        let (code, out, _) = lancer(&["list", "--all"], sample());
        assert_eq!(code, 0);
        assert!(
            out.contains("interdit : volume système (/dev/nvme0n1p2)"),
            "{out}"
        );
        assert!(out.contains("/dev/nvme0n1p2 monté sur /"), "{out}");
        assert!(out.contains("non affiché (bus interne"), "{out}");
    }

    #[test]
    fn show_internal_propose_le_disque_interne_avec_avertissement() {
        let (_, out, _) = lancer(&["list", "--show-internal"], sample());
        assert!(out.contains("sata-libre"), "{out}");
        assert!(out.contains("confirmation renforcée"), "{out}");
        assert!(!out.contains("nvme-systeme"), "{out}");
    }

    #[test]
    fn aucun_disque_donne_un_message_et_le_code_zero() {
        let (code, out, _) = lancer(&["list"], vec![]);
        assert_eq!(code, 0);
        assert!(out.contains("Aucun disque proposé."), "{out}");
        assert!(out.contains("--all"), "{out}");
    }

    #[test]
    fn colonnes_alignees() {
        let (_, out, _) = lancer(&["list", "--all"], sample());
        let columns: Vec<usize> = out
            .lines()
            .filter_map(|l| l.find("/dev/").or_else(|| l.find("CHEMIN")))
            .collect();
        assert_eq!(columns.len(), 4, "{out}");
        assert!(
            columns.windows(2).all(|w| w[0] == w[1]),
            "{columns:?}\n{out}"
        );
    }

    #[test]
    fn json_est_valide_et_versionne() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["list", "--json", "--all"], sample());
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["schema_version"], 1);
        let disks = json["disks"].as_array().map_or(0, Vec::len);
        assert_eq!(disks, 3);
        assert_eq!(json["disks"][0]["id"], "nvme-systeme");
        assert_eq!(json["disks"][0]["eligibility"]["verdict"], "excluded");
        assert_eq!(json["disks"][1]["eligibility"]["verdict"], "offered");
        assert_eq!(json["disks"][2]["eligibility"]["verdict"], "hidden");
        Ok(())
    }

    #[test]
    fn json_par_defaut_ne_contient_que_les_disques_proposes() -> Result<(), serde_json::Error> {
        let (_, out, _) = lancer(&["list", "--json"], sample());
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["disks"].as_array().map_or(0, Vec::len), 1);
        assert_eq!(json["disks"][0]["id"], "usb-cle");
        Ok(())
    }

    #[test]
    fn json_sans_disque_reste_du_json_valide() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["list", "--json"], vec![]);
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["disks"].as_array().map(Vec::len), Some(0));
        Ok(())
    }

    #[test]
    fn echec_de_l_enumeration_donne_le_code_un_et_rien_sur_stdout() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            ["bonk", "list"],
            None,
            &mut out,
            &mut err,
            |_| Ok(()),
            || Err(EnumerateError::Unsupported { os: "test" }),
            || Ok(()),
        );
        assert_eq!(code, EXIT_RUNTIME_ERROR);
        assert!(out.is_empty());
        let err = String::from_utf8_lossy(&err);
        assert!(err.starts_with("bonk : erreur : "), "{err}");
    }

    #[test]
    fn option_inconnue_donne_le_code_d_usage() {
        let (code, out, _) = lancer(&["list", "--efface-tout"], sample());
        assert_eq!(code, EXIT_USAGE);
        assert!(out.is_empty());
    }

    #[test]
    fn disk_list_et_list_sont_equivalents() {
        let court = lancer(&["list", "--all"], sample());
        let long = lancer(&["disk", "list", "--all"], sample());
        assert_eq!(court, long);
        assert_eq!(court.0, 0);
    }

    #[test]
    fn info_par_identifiant_ou_par_chemin() {
        for cible in ["usb-cle", "/dev/sdb"] {
            let (code, out, err) = lancer(&["info", cible], sample());
            assert_eq!((code, err.as_str()), (0, ""), "{cible}");
            assert!(out.contains("Identifiant"), "{out}");
            assert!(out.contains("usb-cle"), "{out}");
            assert!(out.contains("/dev/sdb"), "{out}");
            assert!(out.contains("proposé"), "{out}");
        }
        assert_eq!(
            lancer(&["disk", "info", "usb-cle"], sample()),
            lancer(&["info", "usb-cle"], sample())
        );
    }

    #[test]
    fn info_montre_les_partitions_et_les_motifs() {
        let (code, out, _) = lancer(&["info", "nvme-systeme"], sample());
        assert_eq!(code, 0);
        assert!(out.contains("Partitions :"), "{out}");
        assert!(out.contains("/dev/nvme0n1p2"), "{out}");
        assert!(out.contains("monté sur /"), "{out}");
        assert!(out.contains("interdit : volume système"), "{out}");
    }

    #[test]
    fn info_suit_les_reglages_du_filtre() {
        let (_, defaut, _) = lancer(&["info", "sata-libre"], sample());
        assert!(defaut.contains("non affiché"), "{defaut}");
        let (_, interne, _) = lancer(&["info", "sata-libre", "--show-internal"], sample());
        assert!(interne.contains("confirmation renforcée"), "{interne}");
    }

    #[test]
    fn info_json_est_versionne() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["info", "usb-cle", "--json"], sample());
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["disk"]["id"], "usb-cle");
        assert_eq!(json["disk"]["eligibility"]["verdict"], "offered");
        Ok(())
    }

    #[test]
    fn info_d_un_disque_inconnu_echoue_sans_rien_ecrire_sur_stdout() {
        let (code, out, err) = lancer(&["info", "n-existe-pas"], sample());
        assert_eq!(code, EXIT_RUNTIME_ERROR);
        assert!(out.is_empty());
        assert!(
            err.contains("disque introuvable : « n-existe-pas »"),
            "{err}"
        );
        assert!(err.contains("bonk list --all"), "{err}");
    }

    #[test]
    fn info_exige_une_cible() {
        let (code, _, _) = lancer(&["info"], sample());
        assert_eq!(code, EXIT_USAGE);
    }

    #[test]
    fn diagnose_ecrit_un_rapport_json_sur_la_sortie() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["diagnose"], sample());
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["enumeration"]["ok"], true);
        assert_eq!(
            json["enumeration"]["disks"].as_array().map(Vec::len),
            Some(3)
        );
        Ok(())
    }

    #[test]
    fn diagnose_rapporte_un_echec_d_enumeration_avec_le_code_zero() -> Result<(), serde_json::Error>
    {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            ["bonk", "diagnose"],
            None,
            &mut out,
            &mut err,
            |_| Ok(()),
            || Err(EnumerateError::Unsupported { os: "test" }),
            || Ok(()),
        );
        assert_eq!(code, 0);
        let json: Value = serde_json::from_slice(&out)?;
        assert_eq!(json["enumeration"]["ok"], false);
        Ok(())
    }

    #[test]
    fn decoupage_des_mots_comme_un_shell() {
        assert_eq!(
            split_words("bonk info 'clé usb' --json").ok(),
            Some(vec![
                "bonk".into(),
                "info".into(),
                "clé usb".into(),
                "--json".into()
            ])
        );
        assert_eq!(
            split_words("bonk info 'it'\\''s'").ok(),
            Some(vec!["bonk".into(), "info".into(), "it's".into()])
        );
        assert_eq!(
            split_words("a  ''  b").ok(),
            Some(vec!["a".into(), String::new(), "b".into()])
        );
        assert!(split_words("bonk info 'ouvert").is_err());
    }

    #[test]
    fn une_ligne_valide_est_acceptee_et_une_invalide_refusee() {
        assert!(check_command_line("bonk list --all --show-internal").is_ok());
        assert!(check_command_line("bonk disk info usb-cle").is_ok());
        assert!(check_command_line("bonk efface --tout").is_err());
        assert!(check_command_line("bonk info").is_err());
    }

    #[test]
    fn debug_filter_explique_un_disque_propose() {
        let (code, out, err) = lancer(&["debug", "filter", "usb-cle"], sample());
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(out.contains("§3.2   bus"), "{out}");
        assert!(out.contains("Verdict : proposé"), "{out}");
        assert!(!out.contains("APPLIQUÉE"), "{out}");
    }

    #[test]
    fn debug_filter_explique_un_refus_regle_par_regle() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["debug", "filter", "nvme-systeme"], sample());
        assert_eq!(code, 0);
        assert!(out.contains("[APPLIQUÉE] §3.1.1 system_volume"), "{out}");
        assert!(out.contains("Verdict : interdit"), "{out}");

        let (_, json, _) = lancer(&["debug", "filter", "nvme-systeme", "--json"], sample());
        let value: Value = serde_json::from_str(&json)?;
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["verdict"]["verdict"], "excluded");
        let rules = value["rules"].as_array().map_or(&[][..], Vec::as_slice);
        assert!(
            rules
                .iter()
                .any(|r| r["rule"] == "system_volume" && r["fired"] == true)
        );
        assert!(
            rules
                .iter()
                .any(|r| r["rule"] == "active_esp" && r["fired"] == false)
        );
        Ok(())
    }

    #[test]
    fn debug_filter_suit_les_reglages_et_refuse_un_inconnu() {
        let (_, out, _) = lancer(
            &["debug", "filter", "sata-libre", "--show-internal"],
            sample(),
        );
        assert!(out.contains("disques internes proposés"), "{out}");
        assert!(out.contains("confirmation renforcée"), "{out}");
        let (code, out, err) = lancer(&["debug", "filter", "absent"], sample());
        assert_eq!(code, EXIT_RUNTIME_ERROR);
        assert!(
            out.is_empty() && err.contains("disque introuvable"),
            "{err}"
        );
    }

    /// Régression : `run` tenait le verrou de la sortie d'erreur pendant que la fenêtre était ouverte,
    /// donc tout événement journalisé par un autre fil (énumération des disques, minuteur) bloquait
    /// pour toujours dès qu'on lançait `bonk -v`. Ici, un fil journalise pendant l'ouverture de la fenêtre.
    #[test]
    fn un_autre_fil_peut_journaliser_pendant_que_la_fenetre_est_ouverte() {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let code = run(["bonk", "-v", "gui"], || {
                std::thread::spawn(|| {
                    log::warn!(target: "bonk::gui", "journal depuis un autre fil");
                })
                .join()
                .map_err(|_| "le fil a paniqué".to_owned())
            });
            let _ = tx.send(code == ExitCode::SUCCESS);
        });
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(10)),
            Ok(true),
            "blocage : le journal d'un autre fil attend le verrou de stderr"
        );
    }

    #[test]
    fn sans_commande_la_fenetre_s_ouvre() {
        let r = lancer_avec(&[], None, vec![], Ok(()));
        assert_eq!(r.code, 0);
        assert!(r.fenetre_ouverte && r.journal_installe);
        assert!(r.out.is_empty());
    }

    #[test]
    fn verbeux_sans_commande_ouvre_la_fenetre_avec_le_journal() {
        for args in [&["-v"][..], &["-vv"], &["gui", "-v"]] {
            let r = lancer_avec(args, None, vec![], Ok(()));
            assert_eq!(r.code, 0, "{args:?}");
            assert!(r.fenetre_ouverte && r.journal_installe, "{args:?}");
        }
    }

    #[test]
    fn l_echec_de_la_fenetre_renvoie_vers_la_ligne_de_commande() {
        let r = lancer_avec(&[], None, vec![], Err("aucun affichage".to_owned()));
        assert_eq!(r.code, EXIT_RUNTIME_ERROR);
        assert!(
            r.err.starts_with(
                "bonk : erreur : impossible d'ouvrir l'interface graphique : aucun affichage"
            ),
            "{}",
            r.err
        );
        assert!(r.err.contains("bonk --help"), "{}", r.err);
    }

    #[test]
    fn les_options_de_journal_sont_acceptees_apres_la_commande() {
        let r = lancer_avec(
            &[
                "list",
                "-vv",
                "--log-format",
                "json",
                "--log-file",
                "/tmp/x.log",
            ],
            None,
            sample(),
            Ok(()),
        );
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(r.journal_installe);
    }

    #[test]
    fn un_reglage_de_journal_invalide_est_une_erreur_d_usage() {
        let r = lancer_avec(&["list"], Some("gui=bavard"), sample(), Ok(()));
        assert_eq!(r.code, EXIT_USAGE);
        assert!(r.err.contains("niveau de journal inconnu"), "{}", r.err);
        assert!(!r.journal_installe && r.out.is_empty());
    }

    #[test]
    fn un_journal_impossible_a_installer_est_une_erreur_d_execution() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            ["bonk", "list"],
            None,
            &mut out,
            &mut err,
            |_| Err("journal : ouverture impossible".to_owned()),
            || Ok(vec![]),
            || Ok(()),
        );
        assert_eq!(code, EXIT_RUNTIME_ERROR);
        assert!(out.is_empty());
    }
}
