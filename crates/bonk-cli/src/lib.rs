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
use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

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
    about = "Create bootable media (USB drives and external disks)",
    long_about = None,
    // Lue par `parse` avant l'analyse : aucun champ ne la porte.
    arg(
        clap::Arg::new("no_color")
            .long("no-color")
            .global(true)
            .action(clap::ArgAction::SetTrue)
            .help("No color in the help and usage errors (`NO_COLOR` has the same effect)"),
    ),
)]
struct Cli {
    #[command(flatten)]
    log: LogArgs,

    #[command(flatten)]
    global: fixture::GlobalArgs,

    /// Never open the window: without a command, print the help and fail, for scripts
    #[arg(long)]
    no_gui: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Disks: list and detail
    Disk {
        #[command(subcommand)]
        action: DiskAction,
    },
    /// Downloaded images: list and cleanup
    Cache {
        #[command(subcommand)]
        action: cache::CacheAction,
    },
    /// List the disks Bonk can write (shortcut for `disk list`)
    List(list::ListArgs),
    /// Detail of a disk (shortcut for `disk info`)
    Info(info::InfoArgs),
    /// Write an image to a disk, then verify it (`--simulate`: to an image file)
    Write(WriteArgs),
    /// Compare a disk to an image, writing nothing
    Verify(VerifyArgs),
    /// Repair the GPT of a disk or an image
    RepairGpt(RepairArgs),
    /// Return a disk to storage use: table, partition, filesystem
    Format(format::FormatArgs),
    /// What this host allows, and why not the rest
    Capabilities(format::CapabilitiesArgs),
    /// Support report as JSON: system, rendering, disks and verdicts, secrets masked
    Diagnose(diagnose::DiagnoseArgs),
    /// Low-level disk view, read-only: system sources, step-by-step filter, partition tables
    Debug {
        #[command(subcommand)]
        action: debug::DebugAction,
    },
    /// Explain an error code (`bonk explain E1003`) or list all codes
    Explain(ExplainArgs),
    /// Persistent settings: list, get, set, reset
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// JSON Schema of a command's `--json` output
    Schema(SchemaArgs),
    /// Shell completions: bash, zsh, fish, powershell, elvish
    Completions {
        /// Target shell
        shell: clap_complete::Shell,
    },
    /// Headless service: drive Bonk with JSON on standard input and output, without a window
    Serve,
    /// Open the window (like `bonk` alone)
    Gui,
}

#[derive(Debug, Subcommand)]
enum DiskAction {
    /// List the disks Bonk can write (read-only)
    List(list::ListArgs),
    /// Detail of a disk (read-only)
    Info(info::InfoArgs),
    /// Write an image to a disk, then verify it
    Write(WriteArgs),
    /// Compare a disk to an image, writing nothing
    Verify(VerifyArgs),
    /// Repair the GPT of a disk
    RepairGpt(RepairArgs),
}

/// Ce qu'une commande a d'incompatible avec `--fixture`, le cas échéant : tout ce qui lirait le système
/// réel ou ouvrirait un vrai disque (le bas niveau reste permis sur un fichier image). Aucun cas par défaut : une nouvelle commande ne compile pas tant que
/// sa réponse n'est pas écrite ici, car l'oubli laisserait écrire sur un vrai disque sous `--fixture`.
fn fixture_conflict(command: &Option<Command>) -> Option<&'static str> {
    const DISK: &str = "--fixture only simulates the enumeration: writing to and raw reading of a disk are refused (use --simulate or --dry-run)";
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
        } => Some("debug sources reads the real system: incompatible with --fixture"),
        Command::Diagnose(a) if a.capture() => {
            Some("--capture reads the real system: incompatible with --fixture")
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
                current.push(chars.next().ok_or("trailing backslash")?);
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
/// `--no-color` règle l'affichage de l'analyse elle-même (aide, erreurs d'usage) : il faut le connaître avant
/// d'analyser. `NO_COLOR` et un flux qui n'est pas un terminal sont déjà traités par clap.
fn parse<I, T>(args: I) -> Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let mut command = Cli::command();
    if args.iter().any(|a| a == "--no-color") {
        command = command.color(clap::ColorChoice::Never);
    }
    let matches = command.try_get_matches_from(args)?;
    Cli::from_arg_matches(&matches)
}

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
    let cli = match parse(args) {
        Ok(cli) => cli,
        Err(erreur) => {
            // `print` choisit la sortie adaptée : aide et version sur stdout, erreurs sur stderr.
            // Un échec d'écriture sur ces flux ne doit pas masquer le code de sortie.
            let _ = erreur.print();
            return u8::try_from(erreur.exit_code()).unwrap_or(EXIT_USAGE);
        }
    };

    let no_gui = cli.no_gui;
    let quiet = cli.log.quiet();
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
            let _ = writeln!(err, "{}", Cli::command().render_help());
            report(err, Code::InvalidArgument, "--no-gui requires a command")
        }
        None | Some(Command::Gui) => run_gui(env.privileged, view, err, gui),
        Some(Command::Disk { action }) => match action {
            DiskAction::List(args) => run_list(&args, &view, out, err, enumerate),
            DiskAction::Info(args) => run_info(&args, &view, out, err, enumerate),
            DiskAction::Write(args) => transfer::run_write(&args, &deps, quiet, out, err),
            DiskAction::Verify(args) => transfer::run_verify(&args, &view, &deps, quiet, out, err),
            DiskAction::RepairGpt(args) => transfer::run_repair(&args, &deps, out, err),
        },
        Some(Command::Write(args)) => transfer::run_write(&args, &deps, quiet, out, err),
        Some(Command::Verify(args)) => transfer::run_verify(&args, &view, &deps, quiet, out, err),
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
