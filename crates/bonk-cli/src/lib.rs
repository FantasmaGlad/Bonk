//! Ligne de commande de Bonk : analyse des arguments, sortie texte et JSON, journal, codes de sortie
//! stables (CDC §9).
//!
//! Ce crate ne connaît pas la fenêtre : le binaire lui passe la fonction qui l'ouvre. Ainsi `bonk` sans
//! commande ouvre la fenêtre, et `bonk -v` l'ouvre avec le journal verbeux.

mod config;
mod debug;
mod diagnose;
mod errors;
mod fixture;
mod info;
mod job;
mod list;
mod logging;
mod schema;
mod serve;
mod slot;
mod transfer;

pub use config::ViewDefaults;

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use bonk_hal::{Disk, EnumerateError, enumerate_disks};
use clap::{CommandFactory, Parser, Subcommand};

use crate::config::ConfigAction;
use crate::errors::{Code, EXIT_USAGE, ExplainArgs, report};
use crate::logging::{LogArgs, LogFormat, Settings};
use crate::schema::SchemaArgs;
use crate::transfer::{RepairArgs, VerifyArgs, WriteArgs};

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

    #[command(flatten)]
    global: fixture::GlobalArgs,

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
    /// Écrit une image sur un disque, puis la vérifie (`--simulate` : sur un fichier image)
    Write(WriteArgs),
    /// Compare un disque à une image, sans rien écrire
    Verify(VerifyArgs),
    /// Répare le GPT d'un disque ou d'une image
    RepairGpt(RepairArgs),
    /// Rapport de support en JSON : système, rendu, disques et verdicts, secrets masqués
    Diagnose(diagnose::DiagnoseArgs),
    /// Bas niveau des disques, en lecture seule : sources système, filtre pas à pas, tables de partitions
    Debug {
        #[command(subcommand)]
        action: debug::DebugAction,
    },
    /// Explique un code d'erreur (`bonk explain E1003`) ou liste tous les codes
    Explain(ExplainArgs),
    /// Réglages persistants : list, get, set, reset
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// JSON Schema de la sortie `--json` d'une commande
    Schema(SchemaArgs),
    /// Complétions de shell : bash, zsh, fish, powershell, elvish
    Completions {
        /// Shell visé
        shell: clap_complete::Shell,
    },
    /// Service headless : pilote Bonk en JSON sur l'entrée et la sortie standard, sans fenêtre
    Serve,
    /// Ouvre la fenêtre (comme `bonk` seul)
    Gui,
}

#[derive(Debug, Subcommand)]
enum DiskAction {
    /// Liste les disques que Bonk peut écrire (lecture seule)
    List(list::ListArgs),
    /// Détail d'un disque (lecture seule)
    Info(info::InfoArgs),
    /// Écrit une image sur un disque, puis la vérifie
    Write(WriteArgs),
    /// Compare un disque à une image, sans rien écrire
    Verify(VerifyArgs),
    /// Répare le GPT d'un disque
    RepairGpt(RepairArgs),
}

/// Ce qu'une commande a d'incompatible avec `--fixture`, le cas échéant : tout ce qui lirait le système
/// réel ou ouvrirait un vrai disque.
fn fixture_conflict(command: &Option<Command>) -> Option<&'static str> {
    const DISK: &str = "--fixture simule l'énumération seulement : l'écriture et la lecture brute d'un disque sont refusées (utiliser --simulate ou --dry-run)";
    match command.as_ref()? {
        Command::Write(a)
        | Command::Disk {
            action: DiskAction::Write(a),
        } if a.touches_real_disk() => Some(DISK),
        Command::Verify(a)
        | Command::Disk {
            action: DiskAction::Verify(a),
        } if a.touches_real_disk() => Some(DISK),
        Command::RepairGpt(a)
        | Command::Disk {
            action: DiskAction::RepairGpt(a),
        } if a.touches_real_disk() => Some(DISK),
        Command::Debug {
            action: debug::DebugAction::Sources(_),
        } => Some("debug sources lit le système réel : incompatible avec --fixture"),
        Command::Diagnose(a) if a.capture() => {
            Some("--capture lit le système réel : incompatible avec --fixture")
        }
        _ => None,
    }
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
    G: FnOnce(ViewDefaults) -> Result<(), String>,
{
    // Flux non verrouillés : `execute` ouvre la fenêtre, pendant laquelle d'autres fils journalisent sur
    // la sortie d'erreur. Un verrou tenu pendant toute l'exécution les bloquerait pour toujours
    // (l'énumération des disques de la fenêtre incluse).
    ExitCode::from(execute(
        args,
        &Env::from_process(),
        &mut io::stdout(),
        &mut io::stderr(),
        logging::install,
        enumerate_disks,
        gui,
    ))
}

/// Ce que Bonk lit de l'environnement du processus et demande au système : injectable pour les tests.
#[derive(Default)]
struct Env {
    /// Valeur de `BONK_LOG`.
    bonk_log: Option<String>,
    /// Fichier de configuration (`BONK_CONFIG`, sinon le dossier de configuration du système).
    config_path: Option<PathBuf>,
    /// Variables `BONK_*` (hors `BONK_LOG` et `BONK_CONFIG`) qui surchargent les réglages.
    vars: Vec<(String, String)>,
    /// Accès aux disques et au signal d'annulation.
    io: job::Io,
}

impl Env {
    fn from_process() -> Self {
        let var = |name: &str| std::env::var(name).ok();
        Self {
            bonk_log: var("BONK_LOG"),
            config_path: config::default_path(&var),
            vars: std::env::vars()
                .filter(|(name, _)| name.starts_with("BONK_"))
                .collect(),
            io: job::Io::default(),
        }
    }
}

/// Cœur de [`run`], avec les flux, l'environnement et les dépendances injectés pour les tests.
fn execute<I, T, O, E, L, F, G>(
    args: I,
    env: &Env,
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
    F: Fn() -> Result<Vec<Disk>, EnumerateError>,
    G: FnOnce(ViewDefaults) -> Result<(), String>,
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

    // Réglages enregistrés : un fichier illisible n'empêche pas une commande de lecture (valeurs par
    // défaut, avertissement au journal) ; `bonk config` le signalera lui-même.
    let (stored, config_error) = match env.config_path.as_deref().map(config::load) {
        Some(Ok(stored)) => (stored, None),
        Some(Err(erreur)) => (config::Stored::default(), Some(erreur)),
        None => (config::Stored::default(), None),
    };
    let effective = config::effective(&stored, &env.vars);
    let view = config::view_defaults(&effective);
    let default_format = match config::value_of(&effective, "log.format") {
        Some("json") => LogFormat::Json,
        _ => LogFormat::Text,
    };

    let settings = match Settings::from_args(&cli.log, env.bonk_log.as_deref(), default_format) {
        Ok(settings) => settings,
        Err(erreur) => return report(err, Code::LogSettingInvalid, erreur),
    };
    if let Err(erreur) = install_log(settings) {
        return report(err, Code::LogUnusable, erreur);
    }
    if let Some(erreur) = &config_error {
        log::warn!(target: "bonk::cli", reason = erreur.as_str(); "configuration unreadable, using defaults");
    }
    log::info!(
        target: "bonk::cli",
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH;
        "start"
    );

    // Capture rejouée : l'énumération vient du fichier, et rien ne peut écrire sur un vrai disque.
    let fixture_reads = match cli.global.path().map(fixture::load) {
        Some(Ok(reads)) => Some(reads),
        Some(Err(erreur)) => return report(err, Code::FixtureInvalid, erreur),
        None => None,
    };
    if fixture_reads.is_some()
        && let Some(raison) = fixture_conflict(&cli.command)
    {
        return report(err, Code::FixtureInvalid, raison);
    }
    let fixture_enumerate;
    let enumerate: &dyn Fn() -> Result<Vec<Disk>, EnumerateError> = match &fixture_reads {
        Some(reads) => {
            log::info!(target: "bonk::cli", reads = reads.len(); "replaying a capture instead of reading the system");
            fixture_enumerate = move || bonk_hal::enumerate_disks_from_capture(reads);
            &fixture_enumerate
        }
        None => &enumerate,
    };
    let deps = job::Deps {
        enumerate,
        io: &env.io,
    };
    match cli.command {
        None | Some(Command::Gui) => {
            log::debug!(target: "bonk::cli", "opening the window");
            match gui(view) {
                Ok(()) => 0,
                Err(erreur) => {
                    // Pas d'affichage (SSH, serveur) ou pas de rendu utilisable : la CLI reste là.
                    log::debug!(target: "bonk::cli", reason = erreur.as_str(); "window failed to open");
                    report(
                        err,
                        Code::WindowUnavailable,
                        format!(
                            "impossible d'ouvrir l'interface graphique : {erreur}\n\
                             Utilisez la ligne de commande : bonk --help"
                        ),
                    )
                }
            }
        }
        Some(Command::Disk { action }) => match action {
            DiskAction::List(args) => run_list(&args, &view, out, err, enumerate),
            DiskAction::Info(args) => run_info(&args, &view, out, err, enumerate),
            DiskAction::Write(args) => transfer::run_write(&args, &view, &deps, out, err),
            DiskAction::Verify(args) => transfer::run_verify(&args, &view, &deps, out, err),
            DiskAction::RepairGpt(args) => transfer::run_repair(&args, &view, &deps, out, err),
        },
        Some(Command::Write(args)) => transfer::run_write(&args, &view, &deps, out, err),
        Some(Command::Verify(args)) => transfer::run_verify(&args, &view, &deps, out, err),
        Some(Command::RepairGpt(args)) => transfer::run_repair(&args, &view, &deps, out, err),
        Some(Command::List(args)) => run_list(&args, &view, out, err, enumerate),
        Some(Command::Info(args)) => run_info(&args, &view, out, err, enumerate),
        Some(Command::Config { action }) => {
            log::debug!(target: "bonk::cli", action = format!("{action:?}").as_str(); "command config");
            match config::run(&action, env.config_path.as_deref(), &env.vars, out) {
                Ok(()) => 0,
                Err(config::ConfigError::Invalid(message)) => {
                    report(err, Code::ConfigInvalid, message)
                }
                Err(config::ConfigError::Output(erreur)) => report(err, Code::OutputFailed, erreur),
            }
        }
        Some(Command::Schema(args)) => match schema::run(&args, out) {
            Ok(true) => 0,
            Ok(false) => report(
                err,
                Code::InvalidArgument,
                schema::unknown_message(args.name().unwrap_or_default()),
            ),
            Err(erreur) => report(err, Code::OutputFailed, erreur),
        },
        Some(Command::Completions { shell }) => {
            let mut buffer = Vec::new();
            clap_complete::generate(shell, &mut Cli::command(), "bonk", &mut buffer);
            match out.write_all(&buffer) {
                Ok(()) => 0,
                Err(erreur) => report(err, Code::OutputFailed, erreur),
            }
        }
        Some(Command::Serve) => {
            let enumerate_fn: serve::EnumerateFn = match &fixture_reads {
                Some(reads) => {
                    let reads = reads.clone();
                    Arc::new(move || bonk_hal::enumerate_disks_from_capture(&reads))
                }
                None => Arc::new(enumerate_disks),
            };
            let cfg = serve::Config {
                enumerate: enumerate_fn,
                make_io: Arc::new(|cancel| job::Io::with_cancel(Some(cancel))),
                config_path: env.config_path.clone(),
                vars: env.vars.clone(),
                view,
            };
            serve::serve(io::stdin().lock(), Box::new(io::stdout()), &cfg)
        }
        Some(Command::Debug { action }) => run_debug(action, out, err, enumerate),
        Some(Command::Explain(args)) => match errors::explain(&args, out) {
            Ok(true) => 0,
            Ok(false) => report(
                err,
                Code::InvalidArgument,
                errors::unknown_code_message(args.code().unwrap_or_default()),
            ),
            Err(erreur) => report(err, Code::OutputFailed, erreur),
        },
        Some(Command::Diagnose(args)) => {
            log::debug!(target: "bonk::cli", include_serials = args.include_serials(); "command diagnose");
            let (result, reads) = if args.capture() {
                let (result, reads) = bonk_hal::enumerate_disks_recorded();
                (result, Some(reads))
            } else {
                (enumerate(), None)
            };
            if let Err(erreur) = &result {
                log::debug!(target: "bonk::cli", reason = erreur.to_string().as_str(); "enumeration failed (reported in the report)");
            }
            match diagnose::run(
                &args,
                &diagnose::Environment::system(),
                &result,
                reads.as_deref(),
                out,
            ) {
                Ok(()) => 0,
                Err(erreur) => report(
                    err,
                    Code::OutputFailed,
                    format!("rapport impossible à écrire : {erreur}"),
                ),
            }
        }
    }
}

/// Énumère les disques ; en cas d'échec, écrit l'erreur et renvoie le code de sortie.
fn disks_or_exit<E, F>(err: &mut E, enumerate: F) -> Result<Vec<Disk>, u8>
where
    E: Write,
    F: Fn() -> Result<Vec<Disk>, EnumerateError>,
{
    enumerate().map_err(|erreur| {
        log::debug!(target: "bonk::cli", reason = erreur.to_string().as_str(); "enumeration failed");
        report(err, Code::of_enumeration(&erreur), erreur)
    })
}

fn run_debug<O: Write, E: Write, F>(
    action: debug::DebugAction,
    out: &mut O,
    err: &mut E,
    enumerate: F,
) -> u8
where
    F: Fn() -> Result<Vec<Disk>, EnumerateError>,
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
        Ok(Outcome::Failed(code, message)) => report(err, code, message),
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}

fn run_list<O: Write, E: Write, F>(
    args: &list::ListArgs,
    view: &ViewDefaults,
    out: &mut O,
    err: &mut E,
    enumerate: F,
) -> u8
where
    F: Fn() -> Result<Vec<Disk>, EnumerateError>,
{
    log::debug!(target: "bonk::cli", all = args.all(), json = args.json(); "command list");
    let disks = match disks_or_exit(err, enumerate) {
        Ok(disks) => disks,
        Err(code) => return code,
    };
    match list::run(args, view, &disks, out) {
        Ok(()) => 0,
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}

fn run_info<O: Write, E: Write, F>(
    args: &info::InfoArgs,
    view: &ViewDefaults,
    out: &mut O,
    err: &mut E,
    enumerate: F,
) -> u8
where
    F: Fn() -> Result<Vec<Disk>, EnumerateError>,
{
    log::debug!(target: "bonk::cli", target_disk = args.target(); "command info");
    let disks = match disks_or_exit(err, enumerate) {
        Ok(disks) => disks,
        Err(code) => return code,
    };
    match info::run(args, view, &disks, out) {
        Ok(true) => 0,
        Ok(false) => report(
            err,
            Code::DiskNotFound,
            format!(
                "disque introuvable : « {} » (identifiants : bonk list --all)",
                args.target()
            ),
        ),
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}

#[cfg(test)]
mod job_tests;

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use bonk_hal::{Bus, DiskKind, Partition};
    use serde_json::Value;

    use super::*;
    use crate::errors::EXIT_RUNTIME_ERROR;

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
            &Env {
                bonk_log: env_log.map(str::to_owned),
                ..Env::default()
            },
            &mut out,
            &mut err,
            |_| {
                *journal.borrow_mut() = true;
                Ok(())
            },
            || Ok(disks.clone()),
            |_| {
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
            &Env::default(),
            &mut out,
            &mut err,
            |_| Ok(()),
            || Err(EnumerateError::Unsupported { os: "test" }),
            |_| Ok(()),
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
            &Env::default(),
            &mut out,
            &mut err,
            |_| Ok(()),
            || Err(EnumerateError::Unsupported { os: "test" }),
            |_| Ok(()),
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
            let code = run(["bonk", "-v", "gui"], |_| {
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
    fn toute_erreur_porte_son_code_en_fin_de_message() {
        let (_, _, err) = lancer(&["info", "absent"], sample());
        assert!(err.trim_end().ends_with("[E1003]"), "{err}");
        let r = lancer_avec(&["list"], Some("gui=bavard"), sample(), Ok(()));
        assert!(r.err.trim_end().ends_with("[E5001]"), "{}", r.err);
        let r = lancer_avec(&[], None, vec![], Err("x".to_owned()));
        assert!(r.err.trim_end().ends_with("[E5003]"), "{}", r.err);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            ["bonk", "list"],
            &Env::default(),
            &mut out,
            &mut err,
            |_| Ok(()),
            || Err(EnumerateError::Unsupported { os: "test" }),
            |_| Ok(()),
        );
        assert_eq!(code, 1);
        assert!(
            String::from_utf8_lossy(&err)
                .trim_end()
                .ends_with("[E1002]")
        );
    }

    #[test]
    fn explain_donne_le_sens_d_un_code_et_refuse_un_code_inconnu() {
        let (code, out, _) = lancer(&["explain", "E1003"], vec![]);
        assert_eq!(code, 0);
        assert!(out.starts_with("E1003 : Disque introuvable"), "{out}");
        let (code, out, _) = lancer(&["explain"], vec![]);
        assert_eq!(code, 0);
        assert!(out.lines().count() >= 20, "{out}");
        let (code, out, err) = lancer(&["explain", "E9999"], vec![]);
        assert_eq!(code, EXIT_USAGE);
        assert!(out.is_empty() && err.contains("code inconnu"), "{err}");
        assert!(err.trim_end().ends_with("[E0001]"), "{err}");
    }

    /// Lance une commande avec un fichier de configuration propre au test et des variables injectées.
    fn lancer_config(
        args: &[&str],
        file: &std::path::Path,
        vars: &[(&str, &str)],
        disks: Vec<Disk>,
    ) -> Resultat {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let vues = RefCell::new(None);
        let env = Env {
            bonk_log: None,
            config_path: Some(file.to_path_buf()),
            vars: vars
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            io: job::Io::default(),
        };
        let code = execute(
            std::iter::once("bonk").chain(args.iter().copied()),
            &env,
            &mut out,
            &mut err,
            |_| Ok(()),
            || Ok(disks.clone()),
            |view| {
                *vues.borrow_mut() = Some(view);
                Ok(())
            },
        );
        Resultat {
            code,
            out: String::from_utf8_lossy(&out).into_owned(),
            err: String::from_utf8_lossy(&err).into_owned(),
            journal_installe: false,
            fenetre_ouverte: vues.borrow().is_some(),
        }
    }

    fn fichier_de_config(label: &str) -> std::path::PathBuf {
        std::env::temp_dir()
            .join(format!("bonk-cli-config-{}-{label}", std::process::id()))
            .join("config.json")
    }

    #[test]
    fn config_set_puis_list_puis_effet_sur_list_et_la_fenetre()
    -> Result<(), Box<dyn std::error::Error>> {
        let file = fichier_de_config("effet");
        let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
        let r = lancer_config(
            &["config", "set", "view.show_all", "true"],
            &file,
            &[],
            vec![],
        );
        assert_eq!((r.code, r.err.as_str()), (0, ""), "{}", r.err);
        let r = lancer_config(&["config", "list"], &file, &[], vec![]);
        assert!(
            r.out.contains("view.show_all") && r.out.contains("(fichier)"),
            "{}",
            r.out
        );
        // Sans option, `list` montre maintenant aussi les disques écartés.
        let r = lancer_config(&["list"], &file, &[], sample());
        assert!(r.out.contains("nvme-systeme"), "{}", r.out);
        // La variable d'environnement l'emporte sur le fichier.
        let r = lancer_config(
            &["list"],
            &file,
            &[("BONK_VIEW_SHOW_ALL", "false")],
            sample(),
        );
        assert!(!r.out.contains("nvme-systeme"), "{}", r.out);
        // La fenêtre démarre avec ces réglages.
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let vue = RefCell::new(ViewDefaults::default());
        let env = Env {
            config_path: Some(file.clone()),
            ..Env::default()
        };
        let code = execute(
            ["bonk"],
            &env,
            &mut out,
            &mut err,
            |_| Ok(()),
            || Ok(vec![]),
            |v| {
                *vue.borrow_mut() = v;
                Ok(())
            },
        );
        assert_eq!(code, 0);
        assert!(vue.borrow().show_all && !vue.borrow().show_internal);
        let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
        Ok(())
    }

    #[test]
    fn config_refuse_une_valeur_invalide_avec_le_code_d_usage() {
        let file = fichier_de_config("refus");
        let r = lancer_config(&["config", "set", "lang", "en"], &file, &[], vec![]);
        assert_eq!(r.code, EXIT_USAGE);
        assert!(
            r.err.contains("valeurs : fr") && r.err.trim_end().ends_with("[E5002]"),
            "{}",
            r.err
        );
        assert!(!file.exists());
        let r = lancer_config(&["config", "get", "inconnu"], &file, &[], vec![]);
        assert_eq!(r.code, EXIT_USAGE);
    }

    #[test]
    fn un_fichier_de_config_illisible_n_empeche_pas_list() -> Result<(), Box<dyn std::error::Error>>
    {
        let file = fichier_de_config("casse");
        std::fs::create_dir_all(file.parent().ok_or("parent")?)?;
        std::fs::write(&file, "{ casse")?;
        let r = lancer_config(&["list"], &file, &[], sample());
        assert_eq!(r.code, 0, "{}", r.err);
        assert!(r.out.contains("usb-cle"));
        // `config list` en revanche le dit.
        let r = lancer_config(&["config", "list"], &file, &[], vec![]);
        assert_eq!(r.code, EXIT_USAGE);
        assert!(r.err.contains("illisible"), "{}", r.err);
        let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
        Ok(())
    }

    #[test]
    fn schema_liste_et_decrit() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["schema"], vec![]);
        assert_eq!(code, 0);
        assert!(out.lines().any(|l| l == "list") && out.lines().any(|l| l == "debug-layout"));
        let (code, out, _) = lancer(&["schema", "list"], vec![]);
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["type"], "object");
        let (code, _, err) = lancer(&["schema", "inconnu"], vec![]);
        assert_eq!(code, EXIT_USAGE);
        assert!(
            err.contains("schémas :") && err.trim_end().ends_with("[E0001]"),
            "{err}"
        );
        Ok(())
    }

    /// La sortie réelle de `list --json` respecte son propre schéma : mêmes champs de premier niveau.
    #[test]
    fn la_sortie_json_correspond_a_son_schema() -> Result<(), serde_json::Error> {
        let (_, out, _) = lancer(&["list", "--json", "--all"], sample());
        let real: Value = serde_json::from_str(&out)?;
        let (_, schema, _) = lancer(&["schema", "list"], vec![]);
        let schema: Value = serde_json::from_str(&schema)?;
        let props = schema["properties"]
            .as_object()
            .map(|p| p.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let mut keys: Vec<String> = real
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        let mut props = props;
        props.sort();
        assert_eq!(keys, props);
        Ok(())
    }

    #[test]
    fn completions_pour_chaque_shell() {
        for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
            let (code, out, err) = lancer(&["completions", shell], vec![]);
            assert_eq!(code, 0, "{shell} : {err}");
            assert!(out.contains("bonk") && out.contains("diagnose"), "{shell}");
        }
        let (code, _, _) = lancer(&["completions", "tcsh"], vec![]);
        assert_eq!(code, EXIT_USAGE);
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
            &Env::default(),
            &mut out,
            &mut err,
            |_| Err("journal : ouverture impossible".to_owned()),
            || Ok(vec![]),
            |_| Ok(()),
        );
        assert_eq!(code, EXIT_RUNTIME_ERROR);
        assert!(out.is_empty());
    }
}
