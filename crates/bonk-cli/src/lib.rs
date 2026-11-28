#![cfg_attr(
    test,
    allow(clippy::too_many_lines, reason = "a scenario test reads in one piece")
)]

mod cache;
mod catalog;
#[cfg(test)]
mod cli_tests;
mod command_line;
mod commands;
mod config;
mod debug;
mod diagnose;
mod errors;
mod fetch;
mod fixture;
mod fixture_guard;
mod format;
mod grammar;
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

pub use command_line::{check_args, check_command_line};
pub use config::{ViewDefaults, WindowRequest};
use fixture_guard::fixture_reads;
use grammar::{Cli, Command, DiskAction};

use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use bonk_hal::{Disk, EnumerateError, enumerate_disks};
use bonk_ops::job;
use clap::{CommandFactory, FromArgMatches};

use crate::commands::{
    run_completions, run_config, run_debug, run_diagnose, run_explain, run_gui, run_info, run_list,
    run_schema, run_serve,
};
use crate::errors::{Code, EXIT_USAGE, report};
use crate::logging::{LogFormat, Settings};

/// Returns the process exit code (stable, 0 to 6: `bonk_ops::errors`). `gui` opens the window and returns
/// when it closes.
pub fn run<I, T, G>(args: I, gui: Option<G>) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    G: FnOnce(WindowRequest) -> Result<(), String>,
{
    let windowless = gui.is_none();
    let gui = move |request| match gui {
        Some(open) => open(request),
        None => Err("this version is command line only, without a window".to_owned()),
    };
    // Unlocked streams: `execute` opens the window, while other threads log to stderr. A lock held for the
    // whole run would block them forever, the window's disk enumeration included.
    ExitCode::from(execute(
        args,
        &Env {
            windowless,
            ..Env::from_process()
        },
        &mut io::stdout(),
        &mut io::stderr(),
        logging::install,
        enumerate_disks,
        gui,
    ))
}

#[derive(Default)]
struct Env {
    bonk_log: Option<String>,
    config_path: Option<PathBuf>,
    vars: Vec<(String, String)>,
    io: job::Io,
    catalog: catalog::Access,
    privileged: bool,
    /// Built without the window: with no command, the help is the expected answer.
    windowless: bool,
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
            windowless: false,
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
        None if env.windowless => {
            let _ = writeln!(out, "{}", Cli::command().render_help());
            0
        }
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
            DiskAction::Info(args) => run_info(&args, &view, &env.catalog, out, err, enumerate),
            DiskAction::Write(args) => transfer::run_write(&args, deps, quiet, out, err),
            DiskAction::Verify(args) => transfer::run_verify(&args, &view, deps, quiet, out, err),
            DiskAction::RepairGpt(args) => transfer::run_repair(&args, deps, out, err),
        },
        Some(Command::Write(args)) => transfer::run_write(&args, deps, quiet, out, err),
        Some(Command::Verify(args)) => transfer::run_verify(&args, &view, deps, quiet, out, err),
        Some(Command::RepairGpt(args)) => transfer::run_repair(&args, deps, out, err),
        Some(Command::Format(args)) => format::run_format(&args, deps, out, err),
        Some(Command::Vault { action }) => {
            vault::run(&action, &view, &env.catalog, deps, quiet, out, err)
        }
        Some(Command::Capabilities(args)) => format::run_capabilities(&args, deps, out, err),
        Some(Command::List(args)) => run_list(&args, &view, out, err, enumerate),
        Some(Command::Info(args)) => run_info(&args, &view, &env.catalog, out, err, enumerate),
        Some(Command::Config { action }) => run_config(&action, env, out, err),
        Some(Command::Catalog { action }) => catalog::run(&action, deps, quiet, out, err),
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
