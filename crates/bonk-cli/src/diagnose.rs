//! Sous-commande `bonk diagnose` : un rapport de support unique, en lecture seule (CDC §9.3).
//!
//! Il est fait pour être lu par des outils autant que par des personnes : JSON versionné, champs
//! explicites, motifs identifiés par des codes stables. Numéros de série et nom d'utilisateur sont
//! masqués par défaut, car le rapport est fait pour être envoyé.

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::SystemTime;

use bonk_hal::{Disk, EnumerateError, FilterOptions, evaluate};
use clap::Args;
use serde::Serialize;
use serde_json::Value;

use crate::list::Entry;
use crate::logging::iso8601;

/// Version du format du rapport, incrémentée à chaque changement incompatible.
const SCHEMA_VERSION: u32 = 1;

/// Remplaçant des valeurs masquées.
const SERIAL_MASK: &str = "<serial>";
const USER_MASK: &str = "<user>";

/// Rapport de support (lecture seule, JSON)
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct DiagnoseArgs {
    /// Écrit le rapport dans ce fichier (refuse d'écraser un fichier existant) au lieu de la sortie
    #[arg(long, value_name = "FICHIER")]
    output: Option<PathBuf>,

    /// Garde les numéros de série et le nom d'utilisateur, masqués par défaut
    #[arg(long)]
    include_serials: bool,
}

impl DiagnoseArgs {
    pub(crate) fn include_serials(&self) -> bool {
        self.include_serials
    }
}

#[derive(Serialize)]
struct Report<'a> {
    schema_version: u32,
    generated_at: String,
    bonk: Bonk,
    system: System,
    graphics: Graphics,
    redaction: Redaction,
    enumeration: Enumeration<'a>,
}

#[derive(Serialize)]
struct Bonk {
    version: &'static str,
}

#[derive(Serialize)]
struct System {
    os: &'static str,
    family: &'static str,
    arch: &'static str,
    /// Version du noyau, quand le système la donne.
    kernel: Option<String>,
}

/// Session graphique : on dit si les variables existent, jamais leur valeur.
#[derive(Serialize)]
struct Graphics {
    wayland_display: bool,
    wayland_socket: bool,
    x11_display: bool,
}

#[derive(Serialize)]
struct Redaction {
    serials_and_user_masked: bool,
}

#[derive(Serialize)]
struct Enumeration<'a> {
    ok: bool,
    error: Option<String>,
    /// Tous les disques, avec leur verdict et tous leurs motifs (comme `bonk list --all`).
    disks: Vec<Entry<'a>>,
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
            kernel: kernel_release(),
            user: std::env::var("USER")
                .or_else(|_| std::env::var("LOGNAME"))
                .or_else(|_| std::env::var("USERNAME"))
                .ok(),
            has: |name| std::env::var_os(name).is_some_and(|v| !v.is_empty()),
        }
    }
}

#[cfg(target_os = "linux")]
fn kernel_release() -> Option<String> {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .map(|text| text.trim().to_owned())
}

#[cfg(not(target_os = "linux"))]
fn kernel_release() -> Option<String> {
    None
}

/// Construit le rapport. Un échec d'énumération n'est pas un échec du diagnostic : il y figure.
pub(crate) fn build(
    args: &DiagnoseArgs,
    env: &Environment,
    enumeration: &Result<Vec<Disk>, EnumerateError>,
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
    };
    let mut value = serde_json::to_value(&report)?;
    if !args.include_serials {
        let serials: Vec<&str> = disks
            .iter()
            .filter_map(|disk| disk.serial.as_deref())
            .filter(|serial| serial.chars().count() >= 4)
            .collect();
        redact(&mut value, &serials, env.user.as_deref());
    }
    Ok(value)
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

/// Écrit le rapport sur `out`, ou dans le fichier demandé. Renvoie le message à afficher le cas échéant.
pub(crate) fn run(
    args: &DiagnoseArgs,
    env: &Environment,
    enumeration: &Result<Vec<Disk>, EnumerateError>,
    out: &mut dyn Write,
) -> io::Result<()> {
    let report = build(args, env, enumeration).map_err(io::Error::from)?;
    match &args.output {
        None => {
            serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
            writeln!(out)
        }
        Some(path) => {
            let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
            serde_json::to_writer_pretty(&mut file, &report).map_err(io::Error::from)?;
            writeln!(file)?;
            writeln!(out, "Rapport écrit dans {}", path.display())
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

    #[test]
    fn ecriture_dans_un_fichier_sans_ecraser() -> Result<(), Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!("bonk-diag-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let with_file = DiagnoseArgs {
            output: Some(path.clone()),
            include_serials: false,
        };
        let mut out = Vec::new();
        run(&with_file, &env(), &Ok(vec![disk()]), &mut out)?;
        assert!(String::from_utf8(out)?.starts_with("Rapport écrit dans "));
        let written: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        assert_eq!(written["schema_version"], 1);
        // Un second appel ne remplace pas le fichier.
        let second = run(&with_file, &env(), &Ok(vec![]), &mut Vec::new());
        assert!(second.is_err_and(|e| e.kind() == io::ErrorKind::AlreadyExists));
        std::fs::remove_file(&path)?;
        Ok(())
    }
}
