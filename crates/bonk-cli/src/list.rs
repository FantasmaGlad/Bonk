//! Sous-commande `bonk list` : les disques que Bonk peut écrire, et pourquoi il écarte les autres.
//!
//! La commande ne fait que présenter les verdicts de `bonk_hal::evaluate` : aucune règle de sécurité
//! ne vit ici. Elle est en lecture seule.

use std::io::{self, Write};

use bonk_hal::{Disk, Eligibility, FilterOptions, bus_label, evaluate, format_size, state_label};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ViewDefaults;

/// Version du format de la sortie `--json`, incrémentée à chaque changement incompatible.
const JSON_SCHEMA_VERSION: u32 = 1;

/// Liste les disques que Bonk peut écrire (lecture seule)
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct ListArgs {
    /// Sortie JSON, pour les scripts
    #[arg(long)]
    json: bool,

    /// Affiche aussi les disques écartés ou non affichés, avec la raison
    #[arg(long)]
    all: bool,

    /// Propose aussi les disques internes non système (avertissement renforcé)
    #[arg(long)]
    show_internal: bool,

    /// Mode test : autorise les disques virtuels (loop, zram, machine virtuelle)
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

/// Le rapport `list`, en JSON : même contenu que la sortie `--json`, pour le service.
pub(crate) fn report_value(
    options: FilterOptions,
    all: bool,
    disks: &[Disk],
) -> Result<serde_json::Value, serde_json::Error> {
    let entries: Vec<Entry<'_>> = disks
        .iter()
        .map(|disk| Entry {
            disk,
            eligibility: evaluate(disk, options),
        })
        .filter(|entry| all || entry.eligibility.is_writable())
        .collect();
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
    // Les réglages enregistrés (`bonk config`) valent quand l'option n'est pas donnée.
    let options = FilterOptions {
        show_internal: args.show_internal || defaults.show_internal,
        allow_virtual: args.allow_virtual || defaults.allow_virtual,
    };
    let all = args.all || defaults.show_all;
    let entries: Vec<Entry<'_>> = disks
        .iter()
        .map(|disk| Entry {
            disk,
            eligibility: evaluate(disk, options),
        })
        .filter(|entry| all || entry.eligibility.is_writable())
        .collect();

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
        writeln!(out, "Aucun disque proposé.")?;
        if !all {
            writeln!(
                out,
                "Utilisez --all pour voir les disques écartés et la raison."
            )?;
        }
        return Ok(());
    }

    let header = ["ID", "CHEMIN", "MODELE", "TAILLE", "BUS", "ETAT"];
    let rows: Vec<[String; 6]> = entries
        .iter()
        .map(|entry| {
            [
                entry.disk.id.clone(),
                entry.disk.path.clone(),
                entry.disk.model.clone(),
                format_size(entry.disk.size_bytes),
                bus_label(entry.disk.bus).to_owned(),
                state_label(&entry.eligibility),
            ]
        })
        .collect();

    // La dernière colonne (l'état) est libre : pas de remplissage inutile en fin de ligne.
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
