//! Ligne de commande de Bonk : analyse des arguments, sortie texte et JSON, journal, codes de sortie
//! stables.
//!
//! Ce crate ne connaît pas la fenêtre : le binaire lui passe la fonction qui l'ouvre. Ainsi `bonk` sans
//! commande ouvre la fenêtre, et `bonk -v` l'ouvre avec le journal verbeux.

mod cache;
#[cfg(test)]
mod cli_tests;
mod commands;
mod config;
mod debug;
mod diagnose;
mod errors;
mod fetch;
mod fixture;
mod format;
mod info;
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

use bonk_hal::{Disk, EnumerateError, enumerate_disks};
use bonk_ops::job;
use clap::{Parser, Subcommand};

use crate::commands::{
    run_completions, run_config, run_debug, run_diagnose, run_explain, run_gui, run_info, run_list,
    run_schema, run_serve,
};
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

    /// N'ouvre jamais la fenêtre : sans commande, affiche l'aide et échoue, pour les scripts
    #[arg(long)]
    no_gui: bool,

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
    /// Images téléchargées : liste et nettoyage
    Cache {
        #[command(subcommand)]
        action: cache::CacheAction,
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
    /// Remet un disque en état de stockage : table, partition, système de fichiers
    Format(format::FormatArgs),
    /// Ce que cet hôte permet, et pourquoi pas le reste
    Capabilities(format::CapabilitiesArgs),
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
/// réel ou ouvrirait un vrai disque (le bas niveau reste permis sur un fichier image). Aucun cas par défaut : une nouvelle commande ne compile pas tant que
/// sa réponse n'est pas écrite ici, car l'oubli laisserait écrire sur un vrai disque sous `--fixture`.
fn fixture_conflict(command: &Option<Command>) -> Option<&'static str> {
    const DISK: &str = "--fixture simule l'énumération seulement : l'écriture et la lecture brute d'un disque sont refusées (utiliser --simulate ou --dry-run)";
    match command.as_ref()? {
        Command::Debug {
            action: debug::DebugAction::Layout(a),
        } => (!a.reads_image_file()).then_some(DISK),
        Command::Debug {
            action: debug::DebugAction::Hexdump(a),
        } => (!a.reads_image_file()).then_some(DISK),
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
        Command::Format(a) if a.touches_real_disk() => Some(DISK),
        Command::Debug {
            action: debug::DebugAction::Sources(_),
        } => Some("debug sources lit le système réel : incompatible avec --fixture"),
        Command::Diagnose(a) if a.capture() => {
            Some("--capture lit le système réel : incompatible avec --fixture")
        }
        Command::Format(_)
        | Command::Capabilities(_)
        | Command::Write(_)
        | Command::Verify(_)
        | Command::RepairGpt(_)
        | Command::Disk {
            action:
                DiskAction::Write(_)
                | DiskAction::Verify(_)
                | DiskAction::RepairGpt(_)
                | DiskAction::List(_)
                | DiskAction::Info(_),
        }
        | Command::List(_)
        | Command::Info(_)
        | Command::Diagnose(_)
        | Command::Debug {
            action: debug::DebugAction::Filter(_),
        }
        | Command::Explain(_)
        | Command::Config { .. }
        | Command::Cache { .. }
        | Command::Schema(_)
        | Command::Completions { .. }
        | Command::Serve
        | Command::Gui => None,
    }
}

/// Vérifie qu'une ligne de commande est acceptée par la CLI, sans l'exécuter, et rend ses mots : chaque
/// commande équivalente que la fenêtre affiche doit être valide.
pub fn check_command_line(command: &str) -> Result<Vec<String>, String> {
    let words = split_words(command)?;
    Cli::try_parse_from(&words).map_err(|erreur| erreur.to_string())?;
    Ok(words)
}

/// Découpe une ligne en mots à la façon d'un shell POSIX : espaces, apostrophes simples, guillemets
/// doubles, sans échappement par antislash. Les variables et substitutions ne sont pas interprétées (la fenêtre n'en produit pas).
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

/// Exécute la ligne de commande et renvoie le code de sortie du processus (codes stables, 0 à 6 :
/// `bonk_ops::errors`). `gui` ouvre la fenêtre et rend la main à sa fermeture.
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
    /// Variables `BONK_*` : celles qui portent le nom d'un réglage le surchargent.
    vars: Vec<(String, String)>,
    /// Accès aux disques et au signal d'annulation.
    io: job::Io,
    /// Le processus tourne en administrateur : la fenêtre refuse de s'ouvrir.
    privileged: bool,
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
            privileged: bonk_hal::platform::current().is_privileged(),
        }
    }
}

/// Réglages enregistrés : un fichier illisible n'empêche pas une commande de lecture (valeurs par
/// défaut, avertissement au journal) ; `bonk config` le signalera lui-même.
fn stored_settings(env: &Env) -> (ViewDefaults, LogFormat, Option<String>) {
    let (stored, config_error) = match env.config_path.as_deref().map(config::load) {
        Some(Ok(stored)) => (stored, None),
        Some(Err(erreur)) => (config::Stored::default(), Some(erreur)),
        None => (config::Stored::default(), None),
    };
    let effective = config::effective(&stored, &env.vars);
    let default_format = match config::value_of(&effective, "log.format") {
        Some("json") => LogFormat::Json,
        _ => LogFormat::Text,
    };
    (
        config::view_defaults(&effective),
        default_format,
        config_error,
    )
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

    let no_gui = cli.no_gui;
    let (view, default_format, config_error) = stored_settings(env);

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
        None | Some(Command::Gui) if no_gui => {
            let _ = writeln!(
                err,
                "{}",
                <Cli as clap::CommandFactory>::command().render_help()
            );
            report(err, Code::InvalidArgument, "--no-gui demande une commande")
        }
        None | Some(Command::Gui) => run_gui(env.privileged, view, err, gui),
        Some(Command::Disk { action }) => match action {
            DiskAction::List(args) => run_list(&args, &view, out, err, enumerate),
            DiskAction::Info(args) => run_info(&args, &view, out, err, enumerate),
            DiskAction::Write(args) => transfer::run_write(&args, &deps, out, err),
            DiskAction::Verify(args) => transfer::run_verify(&args, &view, &deps, out, err),
            DiskAction::RepairGpt(args) => transfer::run_repair(&args, &deps, out, err),
        },
        Some(Command::Write(args)) => transfer::run_write(&args, &deps, out, err),
        Some(Command::Verify(args)) => transfer::run_verify(&args, &view, &deps, out, err),
        Some(Command::RepairGpt(args)) => transfer::run_repair(&args, &deps, out, err),
        Some(Command::Format(args)) => format::run_format(&args, &deps, out, err),
        Some(Command::Capabilities(args)) => format::run_capabilities(&args, &deps, out, err),
        Some(Command::List(args)) => run_list(&args, &view, out, err, enumerate),
        Some(Command::Info(args)) => run_info(&args, &view, out, err, enumerate),
        Some(Command::Config { action }) => run_config(&action, env, out, err),
        Some(Command::Cache { action }) => cache::run(&action, out, err),
        Some(Command::Schema(args)) => run_schema(&args, out, err),
        Some(Command::Completions { shell }) => run_completions(shell, out, err),
        Some(Command::Serve) => run_serve(fixture_reads.as_deref(), env, view),
        Some(Command::Debug { action }) => run_debug(action, out, err, enumerate),
        Some(Command::Explain(args)) => run_explain(&args, out, err),
        Some(Command::Diagnose(args)) => run_diagnose(&args, out, err, enumerate),
    }
}

#[cfg(test)]
mod job_tests;
