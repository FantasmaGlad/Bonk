use std::io::{self, Write};

use bonk_hal::{Disk, Eligibility, FilterOptions, bus_label, evaluate, format_size, state_label};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ViewDefaults;

/// Bumped at each incompatible change.
const JSON_SCHEMA_VERSION: u32 = 1;

/// List the disks Bonk can write (read-only)
#[derive(Debug, Args)]
// Without it clap would reuse the doc comment above as long help.
#[command(long_about = None)]
pub(crate) struct ListArgs {
    /// JSON output, for scripts
    #[arg(long)]
    json: bool,

    /// Also show excluded or hidden disks, with the reason
    #[arg(long)]
    all: bool,

    /// Also offer internal non-system disks (stronger warning)
    #[arg(long)]
    show_internal: bool,

    /// Test mode: allow virtual disks (loop, zram, virtual machine)
    #[arg(long)]
    allow_virtual: bool,
}

impl ListArgs {
    pub(crate) fn all(&self) -> bool {
        self.all
    }

    pub(crate) fn json(&self) -> bool {
        self.json
    }
}

#[derive(Serialize, JsonSchema)]
struct Report<'a> {
    schema_version: u32,
    disks: Vec<Entry<'a>>,
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct Entry<'a> {
    #[serde(flatten)]
    pub(crate) disk: &'a Disk,
    pub(crate) eligibility: Eligibility,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Report)).unwrap_or_default()
}

fn selected(disks: &[Disk], options: FilterOptions, all: bool) -> Vec<Entry<'_>> {
    disks
        .iter()
        .map(|disk| Entry {
            disk,
            eligibility: evaluate(disk, options),
        })
        .filter(|entry| all || entry.eligibility.is_writable())
        .collect()
}

pub(crate) fn report_value(
    options: FilterOptions,
    all: bool,
    disks: &[Disk],
) -> Result<serde_json::Value, serde_json::Error> {
    let entries = selected(disks, options, all);
    serde_json::to_value(Report {
        schema_version: JSON_SCHEMA_VERSION,
        disks: entries,
    })
}

pub(crate) fn run(
    args: &ListArgs,
    defaults: &ViewDefaults,
    disks: &[Disk],
    out: &mut dyn Write,
) -> io::Result<()> {
    // Saved settings (`bonk config`) apply when the option is not given.
    let options = defaults.widen(FilterOptions {
        show_internal: args.show_internal,
        allow_virtual: args.allow_virtual,
    });
    let all = args.all || defaults.show_all;
    let entries = selected(disks, options, all);

    if args.json {
        let report = Report {
            schema_version: JSON_SCHEMA_VERSION,
            disks: entries,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)
    } else {
        write_table(&entries, all, out)
    }
}

fn write_table(entries: &[Entry<'_>], all: bool, out: &mut dyn Write) -> io::Result<()> {
    if entries.is_empty() {
        writeln!(out, "No disk offered.")?;
        if !all {
            writeln!(out, "Use --all to see the excluded disks and why.")?;
        }
        return Ok(());
    }

    let header = ["ID", "PATH", "MODEL", "SIZE", "BUS", "STATE"];
    let rows: Vec<[String; 6]> = entries
        .iter()
        .map(|entry| {
            [
                entry.disk.id.clone(),
                entry.disk.path.clone(),
                entry.disk.model.clone(),
                format_size(entry.disk.size_bytes, bonk_hal::Lang::En),
                bus_label(entry.disk.bus, bonk_hal::Lang::En).to_owned(),
                state_label(&entry.eligibility, bonk_hal::Lang::En),
            ]
        })
        .collect();

    // The last column (state) is free: no useless padding at the end of the line.
    let mut widths = header.map(|h| h.chars().count());
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    write_row(out, &header.map(str::to_owned), &widths)?;
    for row in &rows {
        write_row(out, row, &widths)?;
    }
    Ok(())
}

fn write_row(out: &mut dyn Write, cells: &[String; 6], widths: &[usize; 6]) -> io::Result<()> {
    let mut line = String::new();
    for (index, (cell, width)) in cells.iter().zip(widths).enumerate() {
        if index > 0 {
            line.push_str("  ");
        }
        line.push_str(cell);
        if index + 1 < cells.len() {
            line.extend(std::iter::repeat_n(' ', width - cell.chars().count()));
        }
    }
    writeln!(out, "{line}")
}
