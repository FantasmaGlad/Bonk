//! Sous-commande `bonk list` : les disques que Bonk peut écrire, et pourquoi il écarte les autres.
//!
//! La commande ne fait que présenter les verdicts de `bonk_hal::evaluate` : aucune règle de sécurité
//! ne vit ici. Elle est en lecture seule.

use std::io::{self, Write};

use bonk_hal::{
    Bus, Disk, DiskKind, Eligibility, Exclusion, FilterOptions, HiddenReason, Warning, evaluate,
};
use clap::Args;
use serde::Serialize;

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

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    disks: Vec<Entry<'a>>,
}

#[derive(Serialize)]
struct Entry<'a> {
    #[serde(flatten)]
    disk: &'a Disk,
    eligibility: Eligibility,
}

/// Écrit le résultat de `bonk list` sur `out`.
pub(crate) fn run(args: &ListArgs, disks: &[Disk], out: &mut dyn Write) -> io::Result<()> {
    let options = FilterOptions {
        show_internal: args.show_internal,
        allow_virtual: args.allow_virtual,
    };
    let entries: Vec<Entry<'_>> = disks
        .iter()
        .map(|disk| Entry {
            disk,
            eligibility: evaluate(disk, options),
        })
        .filter(|entry| args.all || entry.eligibility.is_writable())
        .collect();

    if args.json {
        let report = Report {
            schema_version: JSON_SCHEMA_VERSION,
            disks: entries,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)
    } else {
        write_table(&entries, args.all, out)
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

/// Taille en unités décimales, comme l'indiquent les fabricants (une clé « 16 Go » fait 16 000 000 000
/// octets ou un peu plus), avec la virgule française.
fn format_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000_000, "To"),
        (1_000_000_000, "Go"),
        (1_000_000, "Mo"),
        (1_000, "Ko"),
    ];
    for (factor, unit) in UNITS {
        if bytes >= factor {
            let tenths = bytes * 10 / factor;
            return format!("{},{} {unit}", tenths / 10, tenths % 10);
        }
    }
    format!("{bytes} o")
}

fn bus_label(bus: Bus) -> &'static str {
    match bus {
        Bus::Usb => "usb",
        Bus::Sd => "sd",
        Bus::Thunderbolt => "thunderbolt",
        Bus::Sata => "sata",
        Bus::Nvme => "nvme",
        Bus::Scsi => "scsi",
        Bus::Virtual => "virtuel",
        Bus::Unknown => "inconnu",
    }
}

fn state_label(eligibility: &Eligibility) -> String {
    match eligibility {
        Eligibility::Offered { warning } => match warning {
            Warning::None => "proposé".to_owned(),
            Warning::InternalDisk => "proposé (disque interne : confirmation renforcée)".to_owned(),
            Warning::VirtualDisk => "proposé (disque virtuel, mode test)".to_owned(),
        },
        Eligibility::Hidden { reason } => match reason {
            HiddenReason::InternalBus => {
                "non affiché (bus interne ; --show-internal pour l'afficher)".to_owned()
            }
        },
        Eligibility::Excluded { reasons } => {
            let texts: Vec<String> = reasons.iter().map(exclusion_label).collect();
            format!("interdit : {}", texts.join(" ; "))
        }
    }
}

fn exclusion_label(reason: &Exclusion) -> String {
    match reason {
        Exclusion::SystemVolume { partition } => format!("volume système ({partition})"),
        Exclusion::ActiveEsp { partition } => format!("partition EFI active ({partition})"),
        Exclusion::Swap { partition } => format!("espace d'échange ({partition})"),
        Exclusion::MountedOutsideUserPaths { partition, mount } => {
            format!("{partition} monté sur {mount}")
        }
        Exclusion::AggregateMember { device, signature } => {
            format!("{device} membre d'un agrégat ({signature})")
        }
        Exclusion::Stacked { device, holders } => {
            format!("{device} utilisé par {}", holders.join(", "))
        }
        Exclusion::VirtualDisk { kind } => {
            let kind = match kind {
                DiskKind::Physical => "physique",
                DiskKind::Loop => "loop",
                DiskKind::Zram => "zram",
                DiskKind::HostVirtual => "machine virtuelle",
            };
            format!("disque virtuel ({kind})")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailles_lisibles() {
        assert_eq!(format_size(0), "0 o");
        assert_eq!(format_size(999), "999 o");
        assert_eq!(format_size(999_999), "999,9 Ko");
        assert_eq!(format_size(1_000_000), "1,0 Mo");
        assert_eq!(format_size(16_008_609_792), "16,0 Go");
        assert_eq!(format_size(512_110_190_592), "512,1 Go");
        assert_eq!(format_size(2_000_398_934_016), "2,0 To");
        // Troncature, jamais d'arrondi vers le haut : une clé ne paraît pas plus grande qu'elle n'est.
        assert_eq!(format_size(15_999_999_999), "15,9 Go");
    }
}
