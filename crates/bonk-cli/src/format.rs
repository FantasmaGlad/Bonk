//! `bonk format` et `bonk capabilities` : présentation ; la logique est dans `bonk_ops::job`.

use std::io::{self, Write};

use bonk_core::format::Filesystem;
use bonk_core::format::table::TableKind;
use bonk_hal::format_size;
use bonk_hal::platform::{Capability, Missing};
use clap::{Args, ValueEnum};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::{Code, report};
use crate::transfer::{CommonArgs, finish, target_line};
use bonk_ops::job::{self, Deps, FormatDone, FormatJob, TraceIo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum FsArg {
    Fat32,
    Exfat,
    Ntfs,
    Ext4,
}

impl From<FsArg> for Filesystem {
    fn from(arg: FsArg) -> Self {
        match arg {
            FsArg::Fat32 => Self::Fat32,
            FsArg::Exfat => Self::Exfat,
            FsArg::Ntfs => Self::Ntfs,
            FsArg::Ext4 => Self::Ext4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TableArg {
    Gpt,
    Mbr,
}

impl From<TableArg> for TableKind {
    fn from(arg: TableArg) -> Self {
        match arg {
            TableArg::Gpt => Self::Gpt,
            TableArg::Mbr => Self::Mbr,
        }
    }
}

/// Remet un disque en état de stockage : une table, une partition, un système de fichiers. Efface tout
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct FormatArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Système de fichiers ; par défaut fat32 jusqu'à 32 Gio, exfat au-delà
    #[arg(long, value_enum)]
    fs: Option<FsArg>,

    /// Table de partitions
    #[arg(long, value_enum, default_value_t = TableArg::Gpt)]
    table: TableArg,

    /// Étiquette du volume
    #[arg(long, value_name = "NOM")]
    label: Option<String>,

    /// Confirme le formatage : sans --yes ni --force, Bonk ne fait rien
    #[arg(long)]
    yes: bool,

    /// Décrit ce qui serait fait sans rien écrire
    #[arg(long)]
    dry_run: bool,

    /// Confirme comme --yes, et autorise un disque interne admis par --show-internal, sans question.
    /// Jamais un disque système : le filtre de sécurité reste souverain
    #[arg(long)]
    force: bool,
}

impl FormatArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate && !self.dry_run
    }
}

pub(crate) fn schema_format() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(FormatDone)).unwrap_or_default()
}

pub(crate) fn run_format(
    args: &FormatArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = FormatJob {
        target: args.common.target(),
        filesystem: args.fs.map(Filesystem::from),
        table: args.table.into(),
        label: args.label.clone(),
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        force: args.force,
        elevation: args.common.elevate.into(),
    };
    let result = job::format(&job, deps);
    finish(result, &args.common, out, err, |done, out| {
        writeln!(
            out,
            "{} {} : {}",
            if done.dry_run {
                "Plan (rien n'est écrit) :"
            } else {
                "Formatage terminé :"
            },
            target_line(&done.target),
            done.target.size_bytes.map_or_else(String::new, format_size),
        )?;
        writeln!(
            out,
            "  Table : {}, système de fichiers : {}{}",
            done.table.name(),
            done.filesystem,
            if done.filesystem_defaulted {
                " (par défaut)"
            } else {
                ""
            }
        )?;
        if let Some(label) = &done.label {
            writeln!(out, "  Étiquette : {label}")?;
        }
        if !done.unmounted.is_empty() {
            writeln!(out, "  Démontage : {}", done.unmounted.join(", "))?;
        }
        if done.verified {
            writeln!(out, "  Relecture : conforme")?;
        }
        Ok(())
    })
}

/// Ce que cet hôte permet, et pourquoi pas le reste
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct CapabilitiesArgs {
    /// Résultat en JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CapabilitiesReport {
    schema_version: u32,
    os: &'static str,
    arch: &'static str,
    capabilities: Vec<Capability>,
}

pub(crate) fn schema_capabilities() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(CapabilitiesReport)).unwrap_or_default()
}

pub(crate) fn report_value(capabilities: Vec<Capability>) -> serde_json::Result<serde_json::Value> {
    serde_json::to_value(CapabilitiesReport {
        schema_version: job::SCHEMA_VERSION,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        capabilities,
    })
}

fn reason(missing: Option<Missing>) -> &'static str {
    match missing {
        Some(Missing::MissingTool) => "outil absent",
        Some(Missing::OsUnsupported) => "non pris en charge",
        Some(Missing::NoPermission) => "droits refusés",
        None => "indisponible",
    }
}

fn write_capabilities(
    args: &CapabilitiesArgs,
    capabilities: Vec<Capability>,
    out: &mut dyn Write,
) -> io::Result<()> {
    if args.json {
        let value = report_value(capabilities).map_err(io::Error::from)?;
        serde_json::to_writer_pretty(&mut *out, &value).map_err(io::Error::from)?;
        return writeln!(out);
    }
    for capability in capabilities {
        if capability.available {
            writeln!(out, "{:<14} oui", capability.name)?;
        } else {
            writeln!(
                out,
                "{:<14} non : {} ({})",
                capability.name,
                reason(capability.missing),
                capability.detail.unwrap_or_default()
            )?;
        }
    }
    Ok(())
}

pub(crate) fn run_capabilities(
    args: &CapabilitiesArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    match write_capabilities(args, (deps.io.capabilities)(), out) {
        Ok(()) => 0,
        Err(erreur) => report(err, Code::OutputFailed, erreur),
    }
}
