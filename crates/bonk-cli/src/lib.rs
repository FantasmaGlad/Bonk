mod cache;
mod catalog;
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
#[cfg(test)]
mod man_tests;
mod schema;
mod serve;
mod slot;
mod transfer;
mod vault;

pub use config::{ViewDefaults, WindowRequest};

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

/// Without a command, the window opens.
#[derive(Debug, Parser)]
// Without it clap reuses the doc comment as long help, and `--help` would show a developer note instead of
// the tool description.
#[command(
    name = "bonk",
    version,
    about = "Create bootable media (USB drives and external disks)",
    long_about = None,
    // Read by `parse` before parsing: no field holds it.
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

#[derive(Debug, clap::Args)]
struct GuiArgs {
    /// Open with this image of the signed catalog chosen (`bonk catalog list`); nothing is written
    #[arg(long, value_name = "ID", conflicts_with = "link")]
    image: Option<String>,
    /// Same, from a `bonk://install/<ID>` link: what the system passes when such a link is opened
    #[arg(long, value_name = "URL")]
    link: Option<String>,
}

impl GuiArgs {
    /// The identifier is checked here in both forms: the window only ever receives a catalog name.
    fn image(&self) -> Result<Option<String>, bonk_catalog::link::LinkError> {
        use bonk_catalog::link::{LinkError, image_id, is_image_id};
        match (&self.image, &self.link) {
            (Some(id), _) if is_image_id(id) => Ok(Some(id.clone())),
            (Some(_), _) => Err(LinkError::InvalidImageId),
            (None, Some(link)) => image_id(link).map(|id| Some(id.to_owned())),
            (None, None) => Ok(None),
        }
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Disks: list and detail
    Disk {
        #[command(subcommand)]
        action: DiskAction,
    },
    /// Images offered by the signed catalog
    Catalog {
        #[command(subcommand)]
        action: catalog::CatalogAction,
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
    /// Disk carrying several ISO images: prepare, list
    Vault {
        #[command(subcommand)]
        action: vault::VaultAction,
    },
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
    Gui(GuiArgs),
    /// Windows: the elevated side of an authorization, started by Bonk itself
    #[command(hide = true)]
    ElevatedHelper {
        #[arg(long)]
        client_pid: u32,
        #[arg(long)]
        pipe: String,
    },
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

/// What a command has that is incompatible with `--fixture`: anything that would read the real system or open
/// a real disk (low level stays allowed on an image file). No default arm: a new command does not compile
/// until its answer is written here, since forgetting it would allow writing to a real disk under
/// `--fixture`.
fn fixture_conflict(command: &Option<Command>) -> Option<&'static str> {
    const DISK: &str = "--fixture only simulates the enumeration: writing to and raw reading of a disk are refused (use --simulate or --dry-run)";
    const GUI: &str = "the window enumerates and writes to real disks: incompatible with --fixture";
    let Some(command) = command else {
        return Some(GUI);
    };
    match command {
        Command::Gui(_) => Some(GUI),
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
        Command::Vault { action } if action.touches_real_disk() => Some(DISK),
        Command::ElevatedHelper { .. } => Some(DISK),
        Command::Debug {
            action: debug::DebugAction::Sources(_),
        } => Some("debug sources reads the real system: incompatible with --fixture"),
        Command::Diagnose(a) if a.capture() => {
            Some("--capture reads the real system: incompatible with --fixture")
        }
        // The service writes, formats and repairs the disks it enumerates: it would judge the captured ones
        // and open real disks of the same name.
        Command::Serve => {
            Some("bonk serve writes to the disks it lists: incompatible with --fixture")
        }
        Command::Format(_)
        | Command::Vault { .. }
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
        | Command::Catalog { .. }
        | Command::Schema(_)
        | Command::Completions { .. } => None,
    }
}

/// Checks a command line is accepted by the CLI without running it, and returns its words: every equivalent
/// command the window shows must be valid.
pub fn check_command_line(command: &str) -> Result<Vec<String>, String> {
    let words = split_words(command)?;
    Cli::try_parse_from(&words).map_err(|erreur| erreur.to_string())?;
    Ok(words)
}

/// POSIX-shell-like splitting: spaces, single quotes, double quotes. Variables and substitutions are not
/// interpreted (the window produces none).
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
            // Outside quotes a backslash makes the next character literal (`'\''` in the text of an id
            // containing an apostrophe).
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
        return Err("unclosed quote".to_owned());
    }
    if in_word {
        words.push(current);
    }
    Ok(words)
}

/// Returns the process exit code (stable, 0 to 6: `bonk_ops::errors`). `gui` opens the window and returns
/// when it closes.
pub fn run<I, T, G>(args: I, gui: G) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    G: FnOnce(WindowRequest) -> Result<(), String>,
{
    // Unlocked streams: `execute` opens the window, while other threads log to stderr. A lock held for the
    // whole run would block them forever, the window's disk enumeration included.
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

/// What Bonk reads from the process environment and asks of the system: injectable for tests.
#[derive(Default)]
struct Env {
    bonk_log: Option<String>,
    /// `BONK_CONFIG`, else the system configuration directory.
    config_path: Option<PathBuf>,
    /// `BONK_*` variables: those named after a setting override it.
    vars: Vec<(String, String)>,
    io: job::Io,
    catalog: catalog::Access,
    /// Running as administrator: the window refuses to open.
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
            catalog: catalog::Access::default(),
            privileged: bonk_hal::platform::current().is_privileged(),
        }
    }
}

/// An unreadable file does not stop a read command (defaults, warning in the log); `bonk config` reports it
/// itself.
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

/// `--no-color` sets the parser's own output (help, usage errors), so it must be known before parsing.
/// `NO_COLOR` and a non-terminal stream are already handled by clap.
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

/// What remains once the arguments are read, the log installed and any capture loaded.
struct Prepared {
    command: Option<Command>,
    no_gui: bool,
    quiet: bool,
    view: ViewDefaults,
    fixture_reads: Option<Vec<bonk_hal::SourceRead>>,
}

/// `Err` carries the exit code.
fn prepare<I, T, E, L>(args: I, env: &Env, err: &mut E, install_log: L) -> Result<Prepared, u8>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    E: Write,
    L: FnOnce(Settings) -> Result<(), String>,
{
    let cli = match parse(args) {
        Ok(cli) => cli,
        Err(erreur) => {
            // `print` picks the right stream: help and version on stdout, errors on stderr. A write failure
            // there must not hide the exit code.
            let _ = erreur.print();
            return Err(u8::try_from(erreur.exit_code()).unwrap_or(EXIT_USAGE));
        }
    };
    if let Some(proxy) = cli.global.proxy(&env.vars)
        && let Err(erreur) = bonk_catalog::set_proxy(&proxy)
    {
        return Err(report(err, Code::InvalidArgument, erreur));
    }
    let (view, default_format, config_error) = stored_settings(env);
    let settings = Settings::from_args(&cli.log, env.bonk_log.as_deref(), default_format)
        .map_err(|erreur| report(err, Code::LogSettingInvalid, erreur))?;
    install_log(settings).map_err(|erreur| report(err, Code::LogUnusable, erreur))?;
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
    let fixture_reads = fixture_reads(&cli, err)?;
    Ok(Prepared {
        quiet: cli.log.quiet(),
        no_gui: cli.no_gui,
        command: cli.command,
        view,
        fixture_reads,
    })
}

/// A replayed capture supplies the enumeration, and no command may then reach a real disk.
fn fixture_reads<E: Write>(
    cli: &Cli,
    err: &mut E,
) -> Result<Option<Vec<bonk_hal::SourceRead>>, u8> {
    let reads = cli
        .global
        .path()
        .map(fixture::load)
        .transpose()
        .map_err(|erreur| report(err, Code::FixtureInvalid, erreur))?;
    if reads.is_some()
        && let Some(raison) = fixture_conflict(&cli.command)
    {
        return Err(report(err, Code::FixtureInvalid, raison));
    }
    Ok(reads)
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
    G: FnOnce(WindowRequest) -> Result<(), String>,
{
    let mut prepared = match prepare(args, env, err, install_log) {
        Ok(prepared) => prepared,
        Err(code) => return code,
    };
    let fixture_reads = prepared.fixture_reads.take();
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
    dispatch(prepared, env, &deps, out, err, gui)
}

fn dispatch<O, E, G>(
    prepared: Prepared,
    env: &Env,
    deps: &job::Deps<'_>,
    out: &mut O,
    err: &mut E,
    gui: G,
) -> u8
where
    O: Write,
    E: Write,
    G: FnOnce(WindowRequest) -> Result<(), String>,
{
    let Prepared {
        command,
        no_gui,
        quiet,
        view,
        ..
    } = prepared;
    let enumerate = deps.enumerate;
    match command {
        None | Some(Command::Gui(_)) if no_gui => {
            let _ = writeln!(err, "{}", Cli::command().render_help());
            report(err, Code::InvalidArgument, "--no-gui requires a command")
        }
        None => run_gui(
            env.privileged,
            WindowRequest { view, image: None },
            err,
            gui,
        ),
        Some(Command::Gui(args)) => match args.image() {
            Ok(image) => run_gui(env.privileged, WindowRequest { view, image }, err, gui),
            Err(erreur) => report(err, Code::InvalidArgument, erreur),
        },
        Some(Command::Disk { action }) => match action {
            DiskAction::List(args) => run_list(&args, &view, out, err, enumerate),
            DiskAction::Info(args) => run_info(&args, &view, out, err, enumerate),
            DiskAction::Write(args) => transfer::run_write(&args, deps, quiet, out, err),
            DiskAction::Verify(args) => transfer::run_verify(&args, &view, deps, quiet, out, err),
            DiskAction::RepairGpt(args) => transfer::run_repair(&args, deps, out, err),
        },
        Some(Command::Write(args)) => transfer::run_write(&args, deps, quiet, out, err),
        Some(Command::Verify(args)) => transfer::run_verify(&args, &view, deps, quiet, out, err),
        Some(Command::RepairGpt(args)) => transfer::run_repair(&args, deps, out, err),
        Some(Command::Format(args)) => format::run_format(&args, deps, out, err),
        Some(Command::Vault { action }) => vault::run(&action, &view, &env.catalog, deps, out, err),
        Some(Command::Capabilities(args)) => format::run_capabilities(&args, deps, out, err),
        Some(Command::List(args)) => run_list(&args, &view, out, err, enumerate),
        Some(Command::Info(args)) => run_info(&args, &view, out, err, enumerate),
        Some(Command::Config { action }) => run_config(&action, env, out, err),
        Some(Command::Catalog { action }) => catalog::run(&action, out, err),
        Some(Command::Cache { action }) => cache::run(&action, out, err),
        Some(Command::Schema(args)) => run_schema(&args, out, err),
        Some(Command::Completions { shell }) => run_completions(shell, out, err),
        Some(Command::ElevatedHelper { client_pid, pipe }) => {
            match bonk_hal::platform::run_elevated_helper(client_pid, &pipe) {
                Ok(()) => 0,
                Err(erreur) => report(err, Code::DeviceOpenDenied, erreur),
            }
        }
        Some(Command::Serve) => run_serve(env, view),
        Some(Command::Debug { action }) => run_debug(action, out, err, enumerate),
        Some(Command::Explain(args)) => run_explain(&args, out, err),
        Some(Command::Diagnose(args)) => run_diagnose(&args, out, err, enumerate),
    }
}

#[cfg(test)]
mod job_tests;
