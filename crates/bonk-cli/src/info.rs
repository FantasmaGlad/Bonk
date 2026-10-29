//! Sous-commande `bonk info <disque>` : le détail d'un disque (série, nature, partitions, étiquettes,
//! montages), équivalent de ce que la fenêtre montre au clic sur un disque (CDC §3.3 et §9.1).

use std::io::{self, Write};

use bonk_hal::{Disk, FilterOptions, bus_label, evaluate, format_size, kind_label, state_label};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;

use crate::config::ViewDefaults;
use crate::list::Entry;

/// Version du format de la sortie `--json`, incrémentée à chaque changement incompatible.
const JSON_SCHEMA_VERSION: u32 = 1;

/// Détail d'un disque (lecture seule)
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct InfoArgs {
    /// Identifiant stable du disque (voir `bonk list --all`) ou son chemin (`/dev/sdb`)
    target: String,

    /// Sortie JSON, pour les scripts
    #[arg(long)]
    json: bool,

    /// Évalue le disque en proposant aussi les disques internes non système
    #[arg(long)]
    show_internal: bool,

    /// Évalue le disque en mode test : disques virtuels autorisés
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

/// Le disque désigné : l'identifiant stable d'abord, le chemin en repli (pratique en lecture seule ;
/// les commandes d'écriture n'accepteront que l'identifiant, CDC §3.3).
pub(crate) fn find<'a>(disks: &'a [Disk], target: &str) -> Option<&'a Disk> {
    disks
        .iter()
        .find(|disk| disk.id == target)
        .or_else(|| disks.iter().find(|disk| disk.path == target))
}

/// Écrit le détail du disque sur `out`. Renvoie `false` si le disque n'existe pas (rien n'est écrit).
pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Report)).unwrap_or_default()
}

/// Le rapport `info`, en JSON, pour le service ; `None` si aucun disque ne répond à `target`.
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
        FilterOptions {
            show_internal: args.show_internal || defaults.show_internal,
            allow_virtual: args.allow_virtual || defaults.allow_virtual,
        },
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

    let rows = [
        ("Modèle", disk.model.clone()),
        ("Identifiant", disk.id.clone()),
        ("Chemin", disk.path.clone()),
        (
            "Série",
            disk.serial.clone().unwrap_or_else(|| "inconnue".to_owned()),
        ),
        (
            "Taille",
            format!(
                "{} ({} octets)",
                format_size(disk.size_bytes),
                disk.size_bytes
            ),
        ),
        ("Bus", bus_label(disk.bus).to_owned()),
        ("Nature", kind_label(disk.kind).to_owned()),
        (
            "Drapeau amovible du système",
            if disk.removable_flag { "oui" } else { "non" }.to_owned(),
        ),
        ("État", state_label(&eligibility)),
    ];
    let width = rows
        .iter()
        .map(|(label, _)| label.chars().count())
        .max()
        .unwrap_or(0);
    for (label, value) in rows {
        writeln!(out, "{label:<width$}  {value}")?;
    }
    if let Some(signature) = &disk.fs_type {
        writeln!(out, "{:<width$}  {signature}", "Signature du disque")?;
    }
    if !disk.holders.is_empty() {
        writeln!(
            out,
            "{:<width$}  {}",
            "Utilisé par",
            disk.holders.join(", ")
        )?;
    }
    if disk.partitions.is_empty() {
        writeln!(out, "Aucune partition détectée.")?;
    } else {
        writeln!(out, "Partitions :")?;
        for partition in &disk.partitions {
            let mounts = if partition.mount_points.is_empty() {
                "non monté".to_owned()
            } else {
                format!("monté sur {}", partition.mount_points.join(", "))
            };
            writeln!(
                out,
                "  {}  {}  {}  étiquette : {}  {mounts}",
                partition.path,
                format_size(partition.size_bytes),
                partition
                    .fs_type
                    .as_deref()
                    .unwrap_or("système de fichiers inconnu"),
                partition.label.as_deref().unwrap_or("aucune"),
            )?;
        }
    }
    Ok(true)
}
