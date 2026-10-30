//! `bonk debug layout` : MBR et GPT décodés, avec leurs constats.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bonk_core::partition::{Layout, Severity, inspect};
use bonk_core::{BlockDevice, DEFAULT_BLOCK_SIZE};
use bonk_hal::{Access, Elevation, FileDevice};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;

use super::{JSON_SCHEMA_VERSION, LayoutArgs, Outcome};

/// Ouvre une image, ou un disque réel désigné par son chemin ou son identifiant stable, en lecture seule.
/// Rien ne l'écrit : le type rendu ne sert ici qu'à lire.
pub(super) fn open_read_only(target: &Path) -> Result<Box<dyn BlockDevice>, (Code, String)> {
    let platform = bonk_hal::platform::current();
    let disk_path = if target.is_file() || target.is_dir() {
        None
    } else if target.exists() {
        Some(target.to_path_buf())
    } else {
        let wanted = target.to_string_lossy();
        platform
            .enumerate()
            .ok()
            .and_then(|disks| disks.into_iter().find(|d| d.id == wanted))
            .map(|d| PathBuf::from(d.path))
    };
    if let Some(path) = disk_path {
        return platform
            .open_disk(&path, Access::ReadOnly, Elevation::Auto)
            .map_err(|erreur| {
                (
                    Code::RawDiskUnavailable,
                    format!("{} : {erreur}", path.display()),
                )
            });
    }
    if !target.exists() && target.to_string_lossy().starts_with("/dev/") {
        return Err((
            Code::RawDiskUnavailable,
            format!("« {} » n'existe pas", target.display()),
        ));
    }
    FileDevice::open(target, Access::ReadOnly, DEFAULT_BLOCK_SIZE)
        .map(|device| Box::new(device) as Box<dyn BlockDevice>)
        .map_err(|erreur| {
            (
                Code::SourceUnreadable,
                format!("{} : {erreur}", target.display()),
            )
        })
}

#[derive(Serialize, JsonSchema)]
pub(super) struct LayoutReport<'a> {
    schema_version: u32,
    #[serde(flatten)]
    layout: &'a Layout,
}

pub(crate) fn schema_layout() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(LayoutReport)).unwrap_or_default()
}

pub(crate) fn layout(args: &LayoutArgs, out: &mut dyn Write) -> io::Result<Outcome> {
    let device = match open_read_only(&args.image) {
        Ok(device) => device,
        Err((code, message)) => return Ok(Outcome::Failed(code, message)),
    };
    let layout = match inspect(&*device) {
        Ok(layout) => layout,
        Err(erreur) => {
            return Ok(Outcome::Failed(
                Code::SourceUnreadable,
                format!("lecture impossible : {erreur}"),
            ));
        }
    };
    if args.json {
        serde_json::to_writer_pretty(
            &mut *out,
            &LayoutReport {
                schema_version: JSON_SCHEMA_VERSION,
                layout: &layout,
            },
        )
        .map_err(io::Error::from)?;
        writeln!(out)?;
        return Ok(Outcome::Done);
    }
    write_layout(&layout, &args.image, out)?;
    Ok(Outcome::Done)
}

pub(super) fn write_layout(layout: &Layout, image: &Path, out: &mut dyn Write) -> io::Result<()> {
    let yes_no = |b: bool| if b { "oui" } else { "NON" };
    writeln!(
        out,
        "Image : {} ({} octets, secteur logique de {} octets)",
        image.display(),
        layout.size_bytes,
        layout.logical_block_size
    )?;
    match &layout.mbr {
        None => writeln!(out, "MBR : aucun")?,
        Some(mbr) => {
            writeln!(
                out,
                "MBR : signature 55 AA {}, protection GPT : {}",
                yes_no(mbr.signature_valid),
                if mbr.protective { "oui" } else { "non" }
            )?;
            for e in &mbr.entries {
                writeln!(
                    out,
                    "  entrée {} : type 0x{:02X}{}  début {}  {} secteurs",
                    e.index + 1,
                    e.kind,
                    if e.bootable { " (amorçable)" } else { "" },
                    e.start_lba,
                    e.sectors
                )?;
            }
        }
    }
    match &layout.gpt {
        None => writeln!(out, "GPT : aucun")?,
        Some(gpt) => {
            for (name, header) in [("principal", &gpt.primary), ("de secours", &gpt.backup)] {
                match header {
                    None => writeln!(out, "GPT {name} : absent")?,
                    Some(h) => writeln!(
                        out,
                        "GPT {name} : secteur {}, révision 0x{:08X}, en-tête {} octets, somme de l'en-tête {}, \
                         somme de la table {}, utilisable {} à {}, table au secteur {} ({} entrées de {} octets), \
                         GUID du disque {}",
                        h.read_at_lba,
                        h.revision,
                        h.header_size,
                        yes_no(h.header_crc_ok),
                        yes_no(h.entries_crc_ok),
                        h.first_usable_lba,
                        h.last_usable_lba,
                        h.entries_lba,
                        h.entry_count,
                        h.entry_size,
                        h.disk_guid
                    )?,
                }
            }
            for e in &gpt.entries {
                let bytes = (e.last_lba.saturating_sub(e.first_lba) + 1)
                    * u64::from(layout.logical_block_size);
                writeln!(
                    out,
                    "  partition {} : secteurs {} à {} ({}), type {}, GUID {}, nom « {} »",
                    e.index + 1,
                    e.first_lba,
                    e.last_lba,
                    bonk_hal::format_size(bytes),
                    e.type_guid,
                    e.unique_guid,
                    e.name
                )?;
            }
        }
    }
    if layout.findings.is_empty() {
        writeln!(out, "Constats : aucun")?;
    } else {
        writeln!(out, "Constats :")?;
        for f in &layout.findings {
            let level = match f.severity {
                Severity::Info => "INFO   ",
                Severity::Warning => "ATTENTION",
                Severity::Error => "ERREUR ",
            };
            writeln!(out, "  [{level}] {} : {}", f.code, f.message)?;
        }
    }
    Ok(())
}
