//! Sous-commandes `bonk debug ...` : le bas niveau des disques, **en lecture seule par construction**
//! (CDC §9.4). Aucune fonction de ce module n'écrit sur un disque ou une image : elles ne touchent
//! que `BlockDevice::read_at` et des lectures de fichiers système.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bonk_core::BlockDevice;
use bonk_core::partition::{Layout, Severity, inspect, read_range};
use bonk_hal::{
    Access, Disk, EnumerateError, FileDevice, FilterOptions, SourceRead, evaluate_traced,
    state_label,
};
use clap::{Args, Subcommand};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;
use crate::info;

const JSON_SCHEMA_VERSION: u32 = 1;
/// Taille d'un dump par défaut, et plafond.
const HEXDUMP_DEFAULT: u64 = 4096;
const HEXDUMP_MAX: u64 = 1 << 20;

#[derive(Debug, Subcommand)]
pub(crate) enum DebugAction {
    /// Chaque source système lue par l'énumération, avec sa durée (et son contenu avec --content)
    Sources(SourcesArgs),
    /// Rejoue le filtre de sécurité sur un disque, règle par règle, avec les faits lus
    Filter(FilterArgs),
    /// Décode MBR, GPT principal et de secours d'une image, et liste les anomalies
    Layout(LayoutArgs),
    /// Octets bruts d'une image, par lignes de 16
    Hexdump(HexdumpArgs),
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct SourcesArgs {
    /// Sortie JSON (contient toujours le contenu des sources)
    #[arg(long)]
    json: bool,
    /// Affiche aussi le contenu de chaque source, tel que lu
    #[arg(long)]
    content: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct FilterArgs {
    /// Identifiant stable du disque ou son chemin
    target: String,
    /// Sortie JSON
    #[arg(long)]
    json: bool,
    /// Évalue avec les disques internes non système proposés
    #[arg(long)]
    show_internal: bool,
    /// Évalue en mode test : disques virtuels autorisés
    #[arg(long)]
    allow_virtual: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct LayoutArgs {
    /// Fichier image à décoder (les disques réels ne sont pas encore lisibles en brut)
    image: PathBuf,
    /// Sortie JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct HexdumpArgs {
    /// Fichier image à lire (les disques réels ne sont pas encore lisibles en brut)
    image: PathBuf,
    /// Décalage de départ en octets (décimal ou 0x...)
    #[arg(long, default_value = "0", value_parser = parse_number)]
    offset: u64,
    /// Nombre d'octets (décimal ou 0x...), 4096 par défaut, 1048576 au plus
    #[arg(long, default_value_t = HEXDUMP_DEFAULT, value_parser = parse_number)]
    len: u64,
}

/// Nombre décimal ou hexadécimal (`0x1BE`).
fn parse_number(text: &str) -> Result<u64, String> {
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("nombre invalide : « {text} » (décimal ou 0x...)"))
}

/// Ce que `debug` a à dire à l'appelant en plus de la sortie.
pub(crate) enum Outcome {
    Done,
    /// Échec expliqué par un message pour la sortie d'erreur.
    Failed(Code, String),
}

// ---------------------------------------------------------------------------------------------
// sources
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
struct SourcesReport<'a> {
    schema_version: u32,
    enumeration: EnumerationSummary,
    reads: &'a [SourceRead],
}

#[derive(Serialize, JsonSchema)]
struct EnumerationSummary {
    ok: bool,
    error: Option<String>,
    disks: usize,
}

pub(crate) fn sources(
    args: &SourcesArgs,
    result: &Result<Vec<Disk>, EnumerateError>,
    reads: &[SourceRead],
    out: &mut dyn Write,
) -> io::Result<Outcome> {
    let summary = EnumerationSummary {
        ok: result.is_ok(),
        error: result.as_ref().err().map(ToString::to_string),
        disks: result.as_ref().map_or(0, Vec::len),
    };
    if args.json {
        let report = SourcesReport {
            schema_version: JSON_SCHEMA_VERSION,
            enumeration: summary,
            reads,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)?;
    } else {
        for read in reads {
            let status = if read.ok { "ok    " } else { "ÉCHEC " };
            let size = read.bytes.map_or_else(String::new, |b| format!("{b} o"));
            writeln!(
                out,
                "[{status}] {:<4} {}  {size}  {} µs",
                read.kind, read.path, read.micros
            )?;
            if let Some(error) = &read.error {
                writeln!(out, "    {error}")?;
            }
            if let (true, Some(content)) = (args.content, &read.content) {
                for line in content.lines() {
                    writeln!(out, "    | {line}")?;
                }
            }
        }
        writeln!(out, "{} lecture(s) de sources.", reads.len())?;
    }
    Ok(match result {
        Ok(_) => Outcome::Done,
        Err(erreur) => Outcome::Failed(
            Code::of_enumeration(erreur),
            format!("l'énumération a échoué : {erreur}"),
        ),
    })
}

// ---------------------------------------------------------------------------------------------
// filter
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, JsonSchema)]
struct FilterReport<'a> {
    schema_version: u32,
    disk: &'a str,
    options: FilterOptions,
    verdict: &'a bonk_hal::Eligibility,
    rules: &'a [bonk_hal::RuleTrace],
}

pub(crate) fn filter(
    args: &FilterArgs,
    disks: &[Disk],
    out: &mut dyn Write,
) -> io::Result<Outcome> {
    let Some(disk) = info::find(disks, &args.target) else {
        return Ok(Outcome::Failed(
            Code::DiskNotFound,
            format!(
                "disque introuvable : « {} » (identifiants : bonk list --all)",
                args.target
            ),
        ));
    };
    let options = FilterOptions {
        show_internal: args.show_internal,
        allow_virtual: args.allow_virtual,
    };
    let (verdict, rules) = evaluate_traced(disk, options);
    if args.json {
        let report = FilterReport {
            schema_version: JSON_SCHEMA_VERSION,
            disk: &disk.id,
            options,
            verdict: &verdict,
            rules: &rules,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)?;
        return Ok(Outcome::Done);
    }
    writeln!(out, "Disque {} ({})", disk.id, disk.path)?;
    writeln!(
        out,
        "Réglages : disques internes {}, disques virtuels {}",
        if options.show_internal {
            "proposés"
        } else {
            "cachés"
        },
        if options.allow_virtual {
            "autorisés"
        } else {
            "interdits"
        },
    )?;
    let subject_width = rules
        .iter()
        .map(|r| r.subject.chars().count())
        .max()
        .unwrap_or(0);
    let rule_width = rules.iter().map(|r| r.rule.len()).max().unwrap_or(0);
    for rule in &rules {
        // La règle du bus ne refuse rien : quand elle « s'applique », elle décide de la proposition.
        let mark = match (rule.rule, rule.fired) {
            ("bus", true) => "VERDICT  ",
            (_, true) => "APPLIQUÉE",
            _ => "non      ",
        };
        writeln!(
            out,
            "[{mark}] §{:<5} {:<rule_width$}  {:<subject_width$}  {}",
            rule.cdc, rule.rule, rule.subject, rule.facts
        )?;
    }
    writeln!(out, "Verdict : {}", state_label(&verdict))?;
    Ok(Outcome::Done)
}

// ---------------------------------------------------------------------------------------------
// layout
// ---------------------------------------------------------------------------------------------

/// Ouvre une image en lecture seule. Un identifiant de disque réel est reconnu pour dire pourquoi il
/// n'est pas lisible, plutôt que d'échouer sur un chemin inexistant.
fn open_image(path: &Path) -> Result<FileDevice, (Code, String)> {
    if !path.exists() && path.to_string_lossy().starts_with("/dev/") {
        return Err((
            Code::RawDiskUnavailable,
            format!(
                "« {} » est un disque réel : sa lecture brute n'est pas encore disponible (étude des \
                 privilèges, D-03) ; passez une image",
                path.display()
            ),
        ));
    }
    FileDevice::open(path, Access::ReadOnly, 512).map_err(|erreur| {
        (
            Code::SourceUnreadable,
            format!("{} : {erreur}", path.display()),
        )
    })
}

#[derive(Serialize, JsonSchema)]
struct LayoutReport<'a> {
    schema_version: u32,
    #[serde(flatten)]
    layout: &'a Layout,
}

pub(crate) fn schema_filter() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(FilterReport)).unwrap_or_default()
}

pub(crate) fn schema_sources() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(SourcesReport)).unwrap_or_default()
}

pub(crate) fn schema_layout() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(LayoutReport)).unwrap_or_default()
}

pub(crate) fn layout(args: &LayoutArgs, out: &mut dyn Write) -> io::Result<Outcome> {
    let device = match open_image(&args.image) {
        Ok(device) => device,
        Err((code, message)) => return Ok(Outcome::Failed(code, message)),
    };
    let layout = match inspect(&device) {
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

fn write_layout(layout: &Layout, image: &Path, out: &mut dyn Write) -> io::Result<()> {
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

// ---------------------------------------------------------------------------------------------
// hexdump
// ---------------------------------------------------------------------------------------------

pub(crate) fn hexdump(args: &HexdumpArgs, out: &mut dyn Write) -> io::Result<Outcome> {
    if args.len == 0 || args.len > HEXDUMP_MAX {
        return Ok(Outcome::Failed(
            Code::InvalidArgument,
            format!(
                "longueur invalide : {} (de 1 à {HEXDUMP_MAX} octets)",
                args.len
            ),
        ));
    }
    let device = match open_image(&args.image) {
        Ok(device) => device,
        Err((code, message)) => return Ok(Outcome::Failed(code, message)),
    };
    let size = device.size();
    if args.offset >= size {
        return Ok(Outcome::Failed(
            Code::InvalidArgument,
            format!(
                "décalage {} au-delà de la fin de l'image ({size} octets)",
                args.offset
            ),
        ));
    }
    // La plage est ramenée à la taille de l'image ; la lecture est alignée par `read_range`.
    let len = args.len.min(size - args.offset);
    let bytes = match read_range(&device, args.offset, len) {
        Ok(bytes) => bytes,
        Err(erreur) => {
            return Ok(Outcome::Failed(
                Code::SourceUnreadable,
                format!("lecture impossible : {erreur}"),
            ));
        }
    };
    for line in hexdump_lines(args.offset, &bytes) {
        writeln!(out, "{line}")?;
    }
    if len < args.len {
        writeln!(
            out,
            "(fin de l'image : {len} octets lus sur {} demandés)",
            args.len
        )?;
    }
    Ok(Outcome::Done)
}

/// Lignes de 16 octets : décalage, deux groupes de 8 octets en hexadécimal, texte ASCII.
pub(crate) fn hexdump_lines(start: u64, bytes: &[u8]) -> Vec<String> {
    bytes
        .chunks(16)
        .enumerate()
        .map(|(row, chunk)| {
            let offset = start + (row as u64) * 16;
            let mut hex = String::new();
            for i in 0..16 {
                if i == 8 {
                    hex.push(' ');
                }
                match chunk.get(i) {
                    Some(b) => hex.push_str(&format!("{b:02x} ")),
                    None => hex.push_str("   "),
                }
            }
            let ascii: String = chunk
                .iter()
                .map(|b| {
                    if b.is_ascii_graphic() || *b == b' ' {
                        *b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{offset:08x}  {hex} |{ascii}|")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombres_decimaux_et_hexadecimaux() {
        assert_eq!(parse_number("512"), Ok(512));
        assert_eq!(parse_number("0x1BE"), Ok(0x1BE));
        assert_eq!(parse_number("0X10"), Ok(16));
        assert!(parse_number("abc").is_err());
        assert!(parse_number("0x").is_err());
        assert!(parse_number("-1").is_err());
    }

    /// Image de test : MBR avec une partition Linux, dans un fichier temporaire supprimé à la fin.
    struct TempImage(PathBuf);

    impl TempImage {
        fn mbr(label: &str) -> Result<Self, io::Error> {
            let path =
                std::env::temp_dir().join(format!("bonk-debug-{}-{label}.img", std::process::id()));
            let mut img = vec![0u8; 4096 * 512];
            img[510] = 0x55;
            img[511] = 0xAA;
            img[446 + 4] = 0x83;
            img[446 + 8..446 + 12].copy_from_slice(&2048u32.to_le_bytes());
            img[446 + 12..446 + 16].copy_from_slice(&1000u32.to_le_bytes());
            std::fs::write(&path, img)?;
            Ok(Self(path))
        }
    }

    impl Drop for TempImage {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn texte(out: Vec<u8>) -> String {
        String::from_utf8_lossy(&out).into_owned()
    }

    #[test]
    fn layout_d_une_image_mbr() -> Result<(), Box<dyn std::error::Error>> {
        let image = TempImage::mbr("layout")?;
        let mut out = Vec::new();
        let args = LayoutArgs {
            image: image.0.clone(),
            json: false,
        };
        assert!(matches!(layout(&args, &mut out)?, Outcome::Done));
        let text = texte(out);
        assert!(
            text.contains("MBR : signature 55 AA oui, protection GPT : non"),
            "{text}"
        );
        assert!(
            text.contains("entrée 1 : type 0x83  début 2048  1000 secteurs"),
            "{text}"
        );
        assert!(text.contains("GPT : aucun"), "{text}");
        assert!(text.contains("Constats : aucun"), "{text}");

        let mut json = Vec::new();
        let args = LayoutArgs {
            image: image.0.clone(),
            json: true,
        };
        layout(&args, &mut json)?;
        let value: serde_json::Value = serde_json::from_slice(&json)?;
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["mbr"]["entries"][0]["start_lba"], 2048);
        assert_eq!(value["logical_block_size"], 512);
        Ok(())
    }

    #[test]
    fn les_commandes_debug_ne_modifient_jamais_l_image() -> Result<(), Box<dyn std::error::Error>> {
        let image = TempImage::mbr("lecture-seule")?;
        let before = std::fs::read(&image.0)?;
        let mtime = std::fs::metadata(&image.0)?.modified()?;
        let mut sink = Vec::new();
        layout(
            &LayoutArgs {
                image: image.0.clone(),
                json: false,
            },
            &mut sink,
        )?;
        layout(
            &LayoutArgs {
                image: image.0.clone(),
                json: true,
            },
            &mut sink,
        )?;
        hexdump(
            &HexdumpArgs {
                image: image.0.clone(),
                offset: 0,
                len: 4096,
            },
            &mut sink,
        )?;
        assert_eq!(std::fs::read(&image.0)?, before);
        assert_eq!(std::fs::metadata(&image.0)?.modified()?, mtime);
        Ok(())
    }

    #[test]
    fn hexdump_borne_et_signale_la_fin_de_l_image() -> Result<(), Box<dyn std::error::Error>> {
        let image = TempImage::mbr("hexdump")?;
        let hex = |offset, len| -> Result<(bool, String), io::Error> {
            let mut out = Vec::new();
            let done = matches!(
                hexdump(
                    &HexdumpArgs {
                        image: image.0.clone(),
                        offset,
                        len
                    },
                    &mut out
                )?,
                Outcome::Done
            );
            Ok((done, texte(out)))
        };
        let (done, text) = hex(0x1c0, 32)?;
        assert!(done && text.starts_with("000001c0  00 00 83"), "{text}");
        // Un décalage non aligné sur le secteur est servi tel quel.
        let (_, text) = hex(0x1fe, 2)?;
        assert!(text.contains("55 aa"), "{text}");
        // Au-delà de la fin : la lecture est ramenée à l'image et le dit.
        let (done, text) = hex(4096 * 512 - 16, 64)?;
        assert!(
            done && text.contains("(fin de l'image : 16 octets lus sur 64 demandés)"),
            "{text}"
        );
        // Décalage hors de l'image et longueurs interdites.
        assert!(!hex(4096 * 512, 16)?.0);
        assert!(!hex(0, 0)?.0);
        assert!(!hex(0, HEXDUMP_MAX + 1)?.0);
        Ok(())
    }

    #[test]
    fn un_disque_reel_est_refuse_avec_une_explication() -> Result<(), io::Error> {
        let mut out = Vec::new();
        let args = LayoutArgs {
            image: PathBuf::from("/dev/disque-qui-n-existe-pas"),
            json: false,
        };
        match layout(&args, &mut out)? {
            Outcome::Failed(code, message) => {
                assert_eq!(code, Code::RawDiskUnavailable);
                assert!(
                    message.contains("lecture brute n'est pas encore disponible"),
                    "{message}"
                );
                assert!(message.contains("D-03"), "{message}");
            }
            Outcome::Done => panic!("un disque réel ne doit pas être lu"),
        }
        assert!(out.is_empty());
        // Un répertoire ou un fichier spécial n'est pas non plus ouvert (FileDevice les refuse).
        let args = LayoutArgs {
            image: std::env::temp_dir(),
            json: false,
        };
        assert!(matches!(
            layout(&args, &mut Vec::new())?,
            Outcome::Failed(Code::SourceUnreadable, _)
        ));
        Ok(())
    }

    #[test]
    fn lignes_de_hexdump() {
        let lines = hexdump_lines(0x1b0, b"EFI PART\x00\x01\x02ABCDEFG");
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            "000001b0  45 46 49 20 50 41 52 54  00 01 02 41 42 43 44 45  |EFI PART...ABCDE|"
        );
        // Ligne incomplète : complétée par des blancs pour garder le texte aligné.
        assert_eq!(
            lines[1],
            "000001c0  46 47                                             |FG|"
        );
        assert!(hexdump_lines(0, &[]).is_empty());
    }
}
