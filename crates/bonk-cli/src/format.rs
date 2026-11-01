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

/// Return a disk to storage use: one table, one partition, one filesystem. Erases everything
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct FormatArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Filesystem; by default fat32 up to 32 GiB, exfat above
    #[arg(long, value_enum)]
    fs: Option<FsArg>,

    /// Partition table
    #[arg(long, value_enum, default_value_t = TableArg::Gpt)]
    table: TableArg,

    /// Volume label
    #[arg(long, value_name = "NAME")]
    label: Option<String>,

    /// Confirm the format: without --yes or --force, Bonk does nothing
    #[arg(long)]
    yes: bool,

    /// Describe what would be done without writing anything
    #[arg(long)]
    dry_run: bool,

    /// Confirm like --yes, and allow an internal disk admitted by --show-internal, without a question.
    /// Never a system disk: the safety filter stays in charge
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
            "{} {}: {}",
            if done.dry_run {
                "Plan (nothing is written):"
            } else {
                "Format finished:"
            },
            target_line(&done.target),
            done.target
                .size_bytes
                .map_or_else(String::new, |s| format_size(s, bonk_hal::Lang::En)),
        )?;
        writeln!(
            out,
            "  Table: {}, filesystem: {}{}",
            done.table.name(),
            done.filesystem,
            if done.filesystem_defaulted {
                " (default)"
            } else {
                ""
            }
        )?;
        if done.file_size_limit_bytes.is_some() {
            writeln!(
                out,
                "  Limit: a file of 4 GiB or more does not fit; to store one, --fs exfat"
            )?;
        }
        if let Some(label) = &done.label {
            writeln!(out, "  Label: {label}")?;
        }
        if !done.unmounted.is_empty() {
            writeln!(out, "  Unmount: {}", done.unmounted.join(", "))?;
        }
        if done.verified {
            writeln!(out, "  Read-back: matches")?;
        }
        Ok(())
    })
}

/// What this host allows, and why not the rest
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct CapabilitiesArgs {
    /// Result as JSON
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
        Some(Missing::MissingTool) => "tool missing",
        Some(Missing::OsUnsupported) => "not supported",
        Some(Missing::NoPermission) => "permission denied",
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
            writeln!(out, "{:<14} yes", capability.name)?;
        } else {
            writeln!(
                out,
                "{:<14} no: {} ({})",
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
