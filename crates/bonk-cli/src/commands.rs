//! Exécution des commandes, une fonction par famille : énumération, sortie et code de sortie.

use std::io::{self, Write};
use std::sync::Arc;

use bonk_hal::{Disk, EnumerateError, SourceRead, enumerate_disks};
use bonk_ops::job;
use clap::CommandFactory;

use crate::config::{self, ConfigAction, ViewDefaults};
use crate::errors::{self, Code, ExplainArgs, report};
use crate::schema::{self, SchemaArgs};
use crate::{Cli, Env, debug, diagnose, info, list, serve};

pub(crate) fn run_gui<E: Write, G>(privileged: bool, view: ViewDefaults, err: &mut E, gui: G) -> u8
where
    G: FnOnce(ViewDefaults) -> Result<(), String>,
{
    if privileged {
        return report(
            err,
            Code::WindowUnavailable,
            "la fenêtre ne s'ouvre pas en administrateur : relancer sans sudo, elle demandera \
             l'autorisation au moment d'écrire ; ou utiliser la ligne de commande (bonk --help)",
        );
    }
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

pub(crate) fn run_config<O: Write, E: Write>(
    action: &ConfigAction,
    env: &Env,
    out: &mut O,
    err: &mut E,
) -> u8 {
    log::debug!(target: "bonk::cli", action = format!("{action:?}").as_str(); "command config");
    match config::run(action, env.config_path.as_deref(), &env.vars, out) {
        Ok(()) => 0,
        Err(config::ConfigError::Invalid(message)) => report(err, Code::ConfigInvalid, message),
        Err(config::ConfigError::Output(erreur)) => report(err, Code::OutputFailed, erreur),
    }
}

pub(crate) fn run_schema<O: Write, E: Write>(args: &SchemaArgs, out: &mut O, err: &mut E) -> u8 {
    match schema::run(args, out) {
        Ok(true) => 0,
        Ok(false) => report(
            err,
            Code::InvalidArgument,
            schema::unknown_message(args.name().unwrap_or_default()),
        ),
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}

pub(crate) fn run_completions<O: Write, E: Write>(
    shell: clap_complete::Shell,
    out: &mut O,
    err: &mut E,
) -> u8 {
    let mut buffer = Vec::new();
    clap_complete::generate(shell, &mut Cli::command(), "bonk", &mut buffer);
    match out.write_all(&buffer) {
        Ok(()) => 0,
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}

pub(crate) fn run_serve(fixture_reads: Option<&[SourceRead]>, env: &Env, view: ViewDefaults) -> u8 {
    let enumerate_fn: serve::EnumerateFn = match fixture_reads {
        Some(reads) => {
            let reads = reads.to_vec();
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

pub(crate) fn run_explain<O: Write, E: Write>(args: &ExplainArgs, out: &mut O, err: &mut E) -> u8 {
    match errors::explain(args, out) {
        Ok(true) => 0,
        Ok(false) => report(
            err,
            Code::InvalidArgument,
            errors::unknown_code_message(args.code().unwrap_or_default()),
        ),
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}

pub(crate) fn run_diagnose<O: Write, E: Write>(
    args: &diagnose::DiagnoseArgs,
    out: &mut O,
    err: &mut E,
    enumerate: &dyn Fn() -> Result<Vec<Disk>, EnumerateError>,
) -> u8 {
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
        args,
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

/// Énumère les disques ; en cas d'échec, écrit l'erreur et renvoie le code de sortie.
pub(crate) fn disks_or_exit<E, F>(err: &mut E, enumerate: F) -> Result<Vec<Disk>, u8>
where
    E: Write,
    F: Fn() -> Result<Vec<Disk>, EnumerateError>,
{
    enumerate().map_err(|erreur| {
        log::debug!(target: "bonk::cli", reason = erreur.to_string().as_str(); "enumeration failed");
        report(err, Code::of_enumeration(&erreur), erreur)
    })
}

pub(crate) fn run_debug<O: Write, E: Write, F>(
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

pub(crate) fn run_list<O: Write, E: Write, F>(
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

pub(crate) fn run_info<O: Write, E: Write, F>(
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
