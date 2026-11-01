//! Sous-commande `bonk diagnose` : un rapport de support unique, en lecture seule.
//!
//! Il est fait pour être lu par des outils autant que par des personnes : JSON versionné, champs
//! explicites, motifs identifiés par des codes stables. Numéros de série (4 caractères ou plus) et
//! nom d'utilisateur (3 ou plus) sont masqués par défaut, car le rapport est fait pour être envoyé.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use bonk_hal::{Disk, EnumerateError, FilterOptions, SourceRead, evaluate};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::list::Entry;
use crate::logging::iso8601;

/// Version du format du rapport, incrémentée à chaque changement incompatible.
const SCHEMA_VERSION: u32 = 1;

/// Remplaçant des valeurs masquées.
const SERIAL_MASK: &str = "<serial>";
const USER_MASK: &str = "<user>";

/// Support report (read-only, JSON)
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct DiagnoseArgs {
    /// Write the report to this file (refuses to overwrite an existing file) instead of the output
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Keep serial numbers and the user name, masked by default
    #[arg(long)]
    include_serials: bool,

    /// Attach the raw enumeration sources (anonymized), replayable elsewhere with --fixture
    #[arg(long)]
    capture: bool,
}

impl DiagnoseArgs {
    pub(crate) fn include_serials(&self) -> bool {
        self.include_serials
    }

    pub(crate) fn capture(&self) -> bool {
        self.capture
    }
}

#[derive(Serialize, JsonSchema)]
struct Report<'a> {
    schema_version: u32,
    generated_at: String,
    bonk: Bonk,
    system: System,
    graphics: Graphics,
    redaction: Redaction,
    enumeration: Enumeration<'a>,
    /// Sources brutes de l'énumération, quand `--capture` est demandé.
    capture: Option<Capture>,
}

/// Lectures des sources système, au format que rejoue `--fixture`.
#[derive(Serialize, JsonSchema)]
struct Capture {
    schema_version: u32,
    kind: &'static str,
    reads: Vec<SourceRead>,
}

#[derive(Serialize, JsonSchema)]
struct Bonk {
    version: &'static str,
}

#[derive(Serialize, JsonSchema)]
struct System {
    os: &'static str,
    family: &'static str,
    arch: &'static str,
    /// Version du noyau, quand le système la donne.
    kernel: Option<String>,
}

/// Session graphique : on dit si les variables existent, jamais leur valeur.
#[derive(Serialize, JsonSchema)]
struct Graphics {
    wayland_display: bool,
    wayland_socket: bool,
    x11_display: bool,
}

#[derive(Serialize, JsonSchema)]
struct Redaction {
    serials_and_user_masked: bool,
}

#[derive(Serialize, JsonSchema)]
struct Enumeration<'a> {
    ok: bool,
    error: Option<String>,
    /// Tous les disques, avec leur verdict et tous leurs motifs (comme `bonk list --all`).
    disks: Vec<Entry<'a>>,
}

pub(crate) fn schema() -> Value {
    serde_json::to_value(schemars::schema_for!(Report)).unwrap_or_default()
}

/// Informations de l'environnement lues par le rapport, injectables pour les tests.
pub(crate) struct Environment {
    pub(crate) now: SystemTime,
    pub(crate) kernel: Option<String>,
    pub(crate) user: Option<String>,
    pub(crate) has: fn(&str) -> bool,
}

impl Environment {
    pub(crate) fn system() -> Self {
        Self {
            now: SystemTime::now(),
            kernel: bonk_hal::platform::current().kernel_release(),
            user: std::env::var("USER")
                .or_else(|_| std::env::var("LOGNAME"))
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            has: |name| std::env::var_os(name).is_some_and(|v| !v.is_empty()),
        }
    }
}

/// Construit le rapport. Un échec d'énumération n'est pas un échec du diagnostic : il y figure.
#[cfg(test)]
pub(crate) fn build(
    args: &DiagnoseArgs,
    env: &Environment,
    enumeration: &Result<Vec<Disk>, EnumerateError>,
) -> Result<Value, serde_json::Error> {
    build_with_capture(args, env, enumeration, None)
}

/// Comme [`build`], avec les lectures de sources à joindre en capture.
pub(crate) fn build_with_capture(
    args: &DiagnoseArgs,
    env: &Environment,
    enumeration: &Result<Vec<Disk>, EnumerateError>,
    reads: Option<&[SourceRead]>,
) -> Result<Value, serde_json::Error> {
    let (ok, error, disks) = match enumeration {
        Ok(disks) => (true, None, disks.as_slice()),
        Err(erreur) => (false, Some(erreur.to_string()), [].as_slice()),
    };
    let report = Report {
        schema_version: SCHEMA_VERSION,
        generated_at: iso8601(env.now),
        bonk: Bonk {
            version: env!("CARGO_PKG_VERSION"),
        },
        system: System {
            os: std::env::consts::OS,
            family: std::env::consts::FAMILY,
            arch: std::env::consts::ARCH,
            kernel: env.kernel.clone(),
        },
        graphics: Graphics {
            wayland_display: (env.has)("WAYLAND_DISPLAY"),
            wayland_socket: (env.has)("WAYLAND_SOCKET"),
            x11_display: (env.has)("DISPLAY"),
        },
        redaction: Redaction {
            serials_and_user_masked: !args.include_serials,
        },
        enumeration: Enumeration {
            ok,
            error,
            disks: disks
                .iter()
                .map(|disk| Entry {
                    disk,
                    eligibility: evaluate(disk, FilterOptions::default()),
                })
                .collect(),
        },
        capture: reads.map(|reads| Capture {
            schema_version: 1,
            kind: "bonk-capture",
            reads: sanitize_reads(reads),
        }),
    };
    let mut value = serde_json::to_value(&report)?;
    if !args.include_serials {
        // Les lectures aussi : une énumération en échec ne rend aucun disque, mais ses lectures restent
        // dans le rapport, avec les numéros dans les liens `by-id`.
        let serials: Vec<&str> = disks
            .iter()
            .filter_map(|disk| disk.serial.as_deref())
            .chain(reads.unwrap_or_default().iter().flat_map(serials_read))
            .filter(|serial| serial.chars().count() >= 4)
            .collect();
        redact(&mut value, &serials, env.user.as_deref());
    }
    Ok(value)
}

/// Clés udev qui identifient un exemplaire de matériel ou un utilisateur : jamais dans une capture.
const UDEV_SENSITIVE_KEYS: [&str; 11] = [
    "ID_SERIAL",
    "ID_SERIAL_SHORT",
    "ID_WWN",
    "ID_WWN_WITH_EXTENSION",
    "ID_FS_UUID",
    "ID_FS_UUID_ENC",
    "ID_FS_UUID_SUB",
    "ID_PART_ENTRY_UUID",
    "ID_PART_TABLE_UUID",
    "ID_FS_LABEL",
    "ID_FS_LABEL_ENC",
];

/// Masque, dans un enregistrement udev, ce qui identifie le disque ou son propriétaire : numéros de
/// série, WWN, UUID, étiquettes et les liens `by-uuid`, `by-partuuid`, `by-label` et `wwn-`. Le reste
/// (bus, signatures, types de partition) est ce qui sert à rejouer l'énumération.
fn sanitize_udev(text: &str) -> String {
    text.lines()
        .map(|line| {
            if let Some(entry) = line.strip_prefix("E:")
                && let Some((key, _)) = entry.split_once('=')
                && UDEV_SENSITIVE_KEYS.contains(&key)
            {
                return format!("E:{key}=<masked>");
            }
            if let Some(link) = line.strip_prefix("S:")
                && [
                    "/by-uuid/",
                    "/by-partuuid/",
                    "/by-label/",
                    "/by-id/wwn-",
                    "eui.",
                ]
                .iter()
                .any(|marker| link.contains(marker))
            {
                let (dir, _) = link.rsplit_once('/').unwrap_or(("", link));
                return format!("S:{dir}/<masked>");
            }
            line.to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Numéros de série qu'une lecture contient en clair.
fn serials_read(read: &SourceRead) -> Vec<&str> {
    let content = read.content.as_deref().unwrap_or_default();
    if read.path.ends_with("/serial") {
        return vec![content.trim()];
    }
    if !read.path.contains("/run/udev/") {
        return Vec::new();
    }
    content
        .lines()
        .filter_map(|line| line.strip_prefix("E:ID_SERIAL_SHORT="))
        .collect()
}

fn sanitize_reads(reads: &[SourceRead]) -> Vec<SourceRead> {
    reads
        .iter()
        .cloned()
        .map(|mut read| {
            if read.path == bonk_hal::windows_facts::SOURCE {
                read.content = read
                    .content
                    .as_deref()
                    .map(bonk_hal::windows_facts::sanitize);
            }
            if read.path == bonk_hal::macos_facts::SOURCE {
                read.content = read.content.as_deref().map(bonk_hal::macos_facts::sanitize);
            }
            if read.kind == "file" && read.path.contains("/run/udev/") {
                read.content = read.content.as_deref().map(sanitize_udev);
            }
            // Sans disque énuméré, `redact` ne connaît aucun numéro : celui-ci se masque d'après sa source.
            if read.kind == "file" && read.path.ends_with("/serial") && read.content.is_some() {
                read.content = Some(SERIAL_MASK.to_owned());
            }
            read
        })
        .collect()
}

/// Masque les numéros de série (partout où ils apparaissent, dans les identifiants `by-id` compris) et
/// le nom d'utilisateur (comme composant de chemin : `/run/media/alice/CLE`). Le nom n'est masqué que
/// comme composant entier, pour ne pas abîmer d'autres mots.
fn redact(value: &mut Value, serials: &[&str], user: Option<&str>) {
    match value {
        Value::String(text) => {
            for serial in serials {
                if text.contains(serial) {
                    *text = text.replace(serial, SERIAL_MASK);
                }
            }
            if let Some(user) = user.filter(|u| u.chars().count() >= 3)
                && text.contains('/')
                && text.split('/').any(|part| part == user)
            {
                *text = text
                    .split('/')
                    .map(|part| if part == user { USER_MASK } else { part })
                    .collect::<Vec<_>>()
                    .join("/");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| redact(v, serials, user)),
        Value::Object(map) => map.values_mut().for_each(|v| redact(v, serials, user)),
        _ => {}
    }
}

/// Le rapport de diagnostic en JSON, pour le service.
pub(crate) fn report_value(
    include_serials: bool,
    env: &Environment,
    enumeration: &Result<Vec<Disk>, EnumerateError>,
) -> Result<Value, serde_json::Error> {
    let args = DiagnoseArgs {
        output: None,
        include_serials,
        capture: false,
    };
    build_with_capture(&args, env, enumeration, None)
}

/// Écrit le rapport sur `out`, ou dans le fichier demandé. Renvoie le message à afficher le cas échéant.
pub(crate) fn run(
    args: &DiagnoseArgs,
    env: &Environment,
    enumeration: &Result<Vec<Disk>, EnumerateError>,
    reads: Option<&[SourceRead]>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let report = build_with_capture(args, env, enumeration, reads).map_err(io::Error::from)?;
    match &args.output {
        None => {
            serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
            writeln!(out)
        }
        Some(path) => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            serde_json::to_writer_pretty(&mut file, &report).map_err(io::Error::from)?;
            writeln!(file)?;
            writeln!(out, "Report written to {}", path.display())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bonk_hal::{Bus, DiskKind, Partition};

    use super::*;

    fn disk() -> Disk {
        Disk {
            id: "usb-SanDisk_Cruzer_4C530001-0:0".to_owned(),
            path: "/dev/sdb".to_owned(),
            model: "SanDisk Cruzer".to_owned(),
            serial: Some("4C530001".to_owned()),
            size_bytes: 16_008_609_792,
            bus: Bus::Usb,
            kind: DiskKind::Physical,
            removable_flag: true,
            fs_type: None,
            holders: vec![],
            partitions: vec![Partition {
                path: "/dev/sdb1".to_owned(),
                label: Some("CLE".to_owned()),
                fs_type: Some("vfat".to_owned()),
                size_bytes: 15_000_000_000,
                mount_points: vec!["/run/media/alice/CLE".to_owned()],
                holders: vec![],
                is_system_volume: false,
                is_active_esp: false,
            }],
        }
    }

    fn env() -> Environment {
        Environment {
            now: std::time::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
            kernel: Some("6.8.0-test".to_owned()),
            user: Some("alice".to_owned()),
            has: |name| name == "DISPLAY",
        }
    }

    fn args(include_serials: bool) -> DiagnoseArgs {
        DiagnoseArgs {
            output: None,
            include_serials,
            capture: false,
        }
    }

    #[test]
    fn rapport_complet_et_versionne() -> Result<(), serde_json::Error> {
        let report = build(&args(false), &env(), &Ok(vec![disk()]))?;
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["generated_at"], "2023-11-14T22:13:20.000Z");
        assert_eq!(report["bonk"]["version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(report["system"]["kernel"], "6.8.0-test");
        assert_eq!(report["graphics"]["x11_display"], true);
        assert_eq!(report["graphics"]["wayland_display"], false);
        assert_eq!(report["enumeration"]["ok"], true);
        assert_eq!(
            report["enumeration"]["disks"][0]["eligibility"]["verdict"],
            "offered"
        );
        assert_eq!(report["redaction"]["serials_and_user_masked"], true);
        Ok(())
    }

    #[test]
    fn serie_et_utilisateur_masques_partout_par_defaut() -> Result<(), serde_json::Error> {
        let report = build(&args(false), &env(), &Ok(vec![disk()]))?;
        let text = report.to_string();
        assert!(!text.contains("4C530001"), "{text}");
        assert!(!text.contains("alice"), "{text}");
        assert_eq!(report["enumeration"]["disks"][0]["serial"], "<serial>");
        assert_eq!(
            report["enumeration"]["disks"][0]["id"],
            "usb-SanDisk_Cruzer_<serial>-0:0"
        );
        assert_eq!(
            report["enumeration"]["disks"][0]["partitions"][0]["mount_points"][0],
            "/run/media/<user>/CLE"
        );
        // Le reste reste lisible : le modèle, les tailles, les étiquettes.
        assert_eq!(report["enumeration"]["disks"][0]["model"], "SanDisk Cruzer");
        assert_eq!(
            report["enumeration"]["disks"][0]["partitions"][0]["label"],
            "CLE"
        );
        Ok(())
    }

    #[test]
    fn include_serials_garde_tout() -> Result<(), serde_json::Error> {
        let report = build(&args(true), &env(), &Ok(vec![disk()]))?;
        let text = report.to_string();
        assert!(
            text.contains("4C530001") && text.contains("alice"),
            "{text}"
        );
        assert_eq!(report["redaction"]["serials_and_user_masked"], false);
        Ok(())
    }

    #[test]
    fn le_nom_d_utilisateur_n_abime_pas_les_autres_mots() -> Result<(), serde_json::Error> {
        let mut with_alias = disk();
        with_alias.model = "alice in wonderland".to_owned();
        with_alias.partitions[0].mount_points =
            vec!["/home/alice2/x".to_owned(), "/srv/alice".to_owned()];
        let report = build(&args(false), &env(), &Ok(vec![with_alias]))?;
        let partition = &report["enumeration"]["disks"][0]["partitions"][0]["mount_points"];
        assert_eq!(partition[0], "/home/alice2/x");
        assert_eq!(partition[1], "/srv/<user>");
        assert_eq!(
            report["enumeration"]["disks"][0]["model"],
            "alice in wonderland"
        );
        Ok(())
    }

    #[test]
    fn un_nom_trop_court_n_est_pas_masque() -> Result<(), serde_json::Error> {
        let mut short = env();
        short.user = Some("al".to_owned());
        let report = build(&args(false), &short, &Ok(vec![disk()]))?;
        assert!(report.to_string().contains("/run/media/alice/CLE"));
        Ok(())
    }

    #[test]
    fn un_echec_d_enumeration_est_rapporte_sans_echouer() -> Result<(), serde_json::Error> {
        let report = build(
            &args(false),
            &env(),
            &Err(EnumerateError::Unsupported { os: "test" }),
        )?;
        assert_eq!(report["enumeration"]["ok"], false);
        assert!(
            report["enumeration"]["error"]
                .as_str()
                .is_some_and(|e| e.contains("test"))
        );
        assert_eq!(
            report["enumeration"]["disks"].as_array().map(Vec::len),
            Some(0)
        );
        Ok(())
    }

    #[test]
    fn le_rapport_ne_contient_aucune_valeur_de_variable_d_environnement()
    -> Result<(), serde_json::Error> {
        let report = build(&args(false), &env(), &Ok(vec![]))?;
        // Seules des présences (booléens) figurent : jamais la valeur de DISPLAY ou autre.
        assert!(
            report["graphics"]
                .as_object()
                .is_some_and(|g| g.values().all(Value::is_boolean))
        );
        Ok(())
    }

    fn udev_read(path: &str, content: &str) -> SourceRead {
        SourceRead {
            path: path.to_owned(),
            kind: "file".to_owned(),
            ok: true,
            bytes: Some(content.len()),
            error: None,
            micros: 1,
            content: Some(content.to_owned()),
        }
    }

    /// Le rapport part chez le support : le masquage ne peut pas dépendre de la réussite de l'énumération.
    #[test]
    fn une_enumeration_en_echec_ne_laisse_pas_passer_de_numero_de_serie()
    -> Result<(), serde_json::Error> {
        let reads = vec![
            udev_read("/sys/block/sda/device/serial", "S4X9NF0M123456\n"),
            udev_read(
                "/run/udev/data/b8:0",
                "S:disk/by-id/usb-SanDisk_Cruzer_4C530001-0:0\nE:ID_SERIAL_SHORT=4C530001\n",
            ),
        ];
        let failed = Err(EnumerateError::Unsupported { os: "test" });
        let text = build_with_capture(&args(false), &env(), &failed, Some(&reads))?.to_string();
        for numero in ["S4X9NF0M123456", "4C530001"] {
            assert!(!text.contains(numero), "numéro laissé en clair");
        }
        Ok(())
    }

    #[test]
    fn la_capture_masque_ce_qui_identifie_le_materiel_et_garde_ce_qui_sert_a_rejouer()
    -> Result<(), serde_json::Error> {
        let udev = "S:disk/by-id/usb-SanDisk_Cruzer_4C530001-0:0\nS:disk/by-id/wwn-0x5000cca\nS:disk/by-uuid/1DCD-D51A\nS:disk/by-label/MA_CLE\nE:ID_SERIAL_SHORT=4C530001\nE:ID_WWN=0x5000cca\nE:ID_FS_UUID=1DCD-D51A\nE:ID_FS_LABEL=MA_CLE\nE:ID_FS_TYPE=vfat\nE:ID_BUS=usb\nE:ID_PART_ENTRY_TYPE=c12a7328\n";
        let reads = vec![
            udev_read("/run/udev/data/b8:17", udev),
            udev_read(
                "/proc/self/mountinfo",
                "90 44 8:17 / /run/media/alice/CLE rw - vfat /dev/sdb1 rw\n",
            ),
        ];
        let report = build_with_capture(&args(false), &env(), &Ok(vec![disk()]), Some(&reads))?;
        let text = report.to_string();
        // Rien d'identifiant n'a survécu.
        for (champ, valeur) in [
            ("numéro de série", "4C530001"),
            ("WWN", "0x5000cca"),
            ("UUID du système de fichiers", "1DCD-D51A"),
            ("étiquette", "MA_CLE"),
            ("nom d'utilisateur", "alice"),
        ] {
            assert!(!text.contains(valeur), "{champ} présent dans la capture");
        }
        let udev_out = report["capture"]["reads"][0]["content"]
            .as_str()
            .ok_or_else(|| serde_json::Error::io(std::io::Error::other("contenu")))?;
        // Ce qui sert à rejouer l'énumération reste.
        for kept in [
            "E:ID_FS_TYPE=vfat",
            "E:ID_BUS=usb",
            "E:ID_PART_ENTRY_TYPE=c12a7328",
            "S:disk/by-id/",
        ] {
            assert!(udev_out.contains(kept), "{kept} perdu : {udev_out}");
        }
        assert_eq!(report["capture"]["kind"], "bonk-capture");
        assert_eq!(
            report["capture"]["reads"][1]["content"],
            "90 44 8:17 / /run/media/<user>/CLE rw - vfat /dev/sdb1 rw\n"
        );
        // Sans --capture, pas de section capture.
        assert!(build(&args(false), &env(), &Ok(vec![]))?["capture"].is_null());
        Ok(())
    }

    #[test]
    fn ecriture_dans_un_fichier_sans_ecraser() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("bonk-diag-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let with_file = DiagnoseArgs {
            output: Some(path.clone()),
            include_serials: false,
            capture: false,
        };
        let mut out = Vec::new();
        run(&with_file, &env(), &Ok(vec![disk()]), None, &mut out)?;
        assert!(String::from_utf8(out)?.starts_with("Report written to "));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        assert_eq!(written["schema_version"], 1);
        // Un second appel ne remplace pas le fichier.
        let second = run(&with_file, &env(), &Ok(vec![]), None, &mut Vec::new());
        assert!(second.is_err_and(|e| e.kind() == io::ErrorKind::AlreadyExists));
        std::fs::remove_file(&path)?;
        Ok(())
    }
}
