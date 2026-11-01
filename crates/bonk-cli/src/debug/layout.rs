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
            .open_disk(
                &path,
                Access::ReadOnly,
                Elevation::Auto,
                &bonk_hal::interrupt::requested,
            )
            .map_err(|erreur| {
                (
                    Code::RawDiskUnavailable,
                    format!("{}: {erreur}", path.display()),
                )
            });
    }
    if !target.exists() && target.to_string_lossy().starts_with("/dev/") {
        return Err((
            Code::RawDiskUnavailable,
            format!("'{}' does not exist", target.display()),
        ));
    }
    FileDevice::open(target, Access::ReadOnly, DEFAULT_BLOCK_SIZE)
        .map(|device| Box::new(device) as Box<dyn BlockDevice>)
        .map_err(|erreur| {
            (
                Code::SourceUnreadable,
                format!("{}: {erreur}", target.display()),
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
                format!("could not read: {erreur}"),
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
    let yes_no = |b: bool| if b { "yes" } else { "NO" };
    writeln!(
        out,
        "Image: {} ({} bytes, logical sector of {} bytes)",
        image.display(),
        layout.size_bytes,
        layout.logical_block_size
    )?;
    match &layout.mbr {
        None => writeln!(out, "MBR: none")?,
        Some(mbr) => {
            writeln!(
                out,
                "MBR: 55 AA signature {}, GPT protection: {}",
                yes_no(mbr.signature_valid),
                if mbr.protective { "yes" } else { "no" }
            )?;
            for e in &mbr.entries {
                writeln!(
                    out,
                    "  entry {}: type 0x{:02X}{}  start {}  {} sectors",
                    e.index + 1,
                    e.kind,
                    if e.bootable { " (bootable)" } else { "" },
                    e.start_lba,
                    e.sectors
                )?;
            }
        }
    }
    match &layout.gpt {
        None => writeln!(out, "GPT: none")?,
        Some(gpt) => {
            for (name, header) in [("primary", &gpt.primary), ("backup", &gpt.backup)] {
                match header {
                    None => writeln!(out, "GPT {name}: missing")?,
                    Some(h) => writeln!(
                        out,
                        "GPT {name}: sector {}, revision 0x{:08X}, header {} bytes, header checksum {}, \
                         table checksum {}, usable {} to {}, table at sector {} ({} entries of {} bytes), \
                         disk GUID {}",
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
                    "  partition {}: sectors {} to {} ({}), type {}, GUID {}, name '{}'",
                    e.index + 1,
                    e.first_lba,
                    e.last_lba,
                    bonk_hal::format_size(bytes, bonk_hal::Lang::En),
                    e.type_guid,
                    e.unique_guid,
                    e.name
                )?;
            }
        }
    }
    if layout.findings.is_empty() {
        writeln!(out, "Findings: none")?;
    } else {
        writeln!(out, "Findings:")?;
        for f in &layout.findings {
            let level = match f.severity {
                Severity::Info => "INFO   ",
                Severity::Warning => "WARNING",
                Severity::Error => "ERROR  ",
            };
            writeln!(out, "  [{level}] {}: {}", f.code, f.message)?;
        }
    }
    Ok(())
}
