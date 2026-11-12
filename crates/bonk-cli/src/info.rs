use std::io::{self, Write};

use bonk_hal::{
    Disk, Eligibility, FilterOptions, bus_label, evaluate, format_size, kind_label, state_label,
};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ViewDefaults;
use crate::list::Entry;

/// Bumped at each incompatible change.
const JSON_SCHEMA_VERSION: u32 = 1;

/// Detail of a disk (read-only)
#[derive(Debug, Args)]
// Without it clap would reuse the doc comment above as long help.
#[command(long_about = None)]
pub(crate) struct InfoArgs {
    /// Stable identifier of the disk (see `bonk list --all`) or its path (`/dev/sdb`)
    target: String,

    /// JSON output, for scripts
    #[arg(long)]
    json: bool,

    /// Evaluate the disk also offering internal non-system disks
    #[arg(long)]
    show_internal: bool,

    /// Evaluate the disk in test mode: virtual disks allowed
    #[arg(long)]
    allow_virtual: bool,
}

impl InfoArgs {
    pub(crate) fn target(&self) -> &str {
        &self.target
    }
}

#[derive(Serialize, JsonSchema)]
struct Report<'a> {
    schema_version: u32,
    disk: Entry<'a>,
}

/// Stable id first, path as a fallback, allowed for reads; write commands only accept the id.
pub(crate) fn find<'a>(disks: &'a [Disk], target: &str) -> Option<&'a Disk> {
    disks
        .iter()
        .find(|disk| disk.id == target)
        .or_else(|| disks.iter().find(|disk| disk.path == target))
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Report)).unwrap_or_default()
}

/// For the service; `None` when no disk matches `target`.
pub(crate) fn report_value(
    target: &str,
    options: FilterOptions,
    disks: &[Disk],
) -> Option<Result<serde_json::Value, serde_json::Error>> {
    let disk = find(disks, target)?;
    let eligibility = evaluate(disk, options);
    Some(serde_json::to_value(Report {
        schema_version: JSON_SCHEMA_VERSION,
        disk: Entry { disk, eligibility },
    }))
}

pub(crate) fn run(
    args: &InfoArgs,
    defaults: &ViewDefaults,
    disks: &[Disk],
    out: &mut dyn Write,
) -> io::Result<bool> {
    let Some(disk) = find(disks, &args.target) else {
        return Ok(false);
    };
    let eligibility = evaluate(
        disk,
        defaults.widen(FilterOptions {
            show_internal: args.show_internal,
            allow_virtual: args.allow_virtual,
        }),
    );
    if args.json {
        let report = Report {
            schema_version: JSON_SCHEMA_VERSION,
            disk: Entry { disk, eligibility },
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)?;
        return Ok(true);
    }
    print_text(disk, &eligibility, out)?;
    Ok(true)
}

fn print_text(disk: &Disk, eligibility: &Eligibility, out: &mut dyn Write) -> io::Result<()> {
    let rows = rows(disk, eligibility);
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    for (label, value) in rows {
        writeln!(out, "{label:<width$}  {value}")?;
    }
    if let Some(signature) = &disk.fs_type {
        writeln!(out, "{:<width$}  {signature}", "Disk signature")?;
    }
    if !disk.holders.is_empty() {
        writeln!(out, "{:<width$}  {}", "Used by", disk.holders.join(", "))?;
    }
    print_partitions(disk, out)
}

fn rows(disk: &Disk, eligibility: &Eligibility) -> [(&'static str, String); 9] {
    [
        ("Model", disk.model.clone()),
        ("Identifier", disk.id.clone()),
        ("Path", disk.path.clone()),
        (
            "Serial",
            disk.serial.clone().unwrap_or_else(|| "unknown".to_owned()),
        ),
        (
            "Size",
            format!(
                "{} ({} bytes)",
                format_size(disk.size_bytes, bonk_hal::Lang::En),
                disk.size_bytes
            ),
        ),
        ("Bus", bus_label(disk.bus, bonk_hal::Lang::En).to_owned()),
        ("Kind", kind_label(disk.kind, bonk_hal::Lang::En).to_owned()),
        (
            "System removable flag",
            if disk.removable_flag { "yes" } else { "no" }.to_owned(),
        ),
        ("State", state_label(eligibility, bonk_hal::Lang::En)),
    ]
}

fn print_partitions(disk: &Disk, out: &mut dyn Write) -> io::Result<()> {
    if disk.partitions.is_empty() {
        writeln!(out, "No partition detected.")?;
    } else {
        writeln!(out, "Partitions:")?;
        for partition in &disk.partitions {
            let mounts = if partition.mount_points.is_empty() {
                "not mounted".to_owned()
            } else {
                format!("mounted on {}", partition.mount_points.join(", "))
            };
            writeln!(
                out,
                "  {}  {}  {}  label: {}  {mounts}",
                partition.path,
                format_size(partition.size_bytes, bonk_hal::Lang::En),
                partition.fs_type.as_deref().unwrap_or("unknown filesystem"),
                partition.label.as_deref().unwrap_or("none"),
            )?;
        }
    }
    Ok(())
}
