//! Ligne de commande de Bonk : analyse des arguments, sortie texte et JSON, codes de sortie stables.
//!
//! Sous-commandes disponibles : `list` (lecture seule). Les autres arrivent avec les jalons suivants.

mod list;

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::ExitCode;

use bonk_hal::{Disk, EnumerateError, enumerate_disks};
use clap::{Parser, Subcommand};

/// Code de sortie d'une erreur d'exécution (énumération impossible, écriture de la sortie en échec).
/// Les erreurs d'usage gardent le code `2` de clap.
const EXIT_RUNTIME_ERROR: u8 = 1;
/// Code de sortie d'une erreur d'usage, celui de clap.
const EXIT_USAGE: u8 = 2;

/// Arguments de la ligne de commande de Bonk.
///
/// Sans argument, l'aide s'affiche : l'interface graphique, lancée dans ce cas à partir du
/// jalon J1, n'existe pas encore.
#[derive(Debug, Parser)]
// `long_about = None` : sans lui, clap reprend ce commentaire de documentation comme texte long
// et `--help` afficherait une note de développeur au lieu de la description de l'outil.
#[command(
    name = "bonk",
    version,
    about = "Création de supports d'amorçage (clés USB et disques externes)",
    long_about = None,
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    List(list::ListArgs),
}

/// Exécute la ligne de commande et renvoie le code de sortie du processus.
///
/// Codes de sortie, stables d'une version à l'autre :
/// - `0` : succès, `--help` et `--version` compris, même si aucun disque n'est proposé ;
/// - `1` : erreur d'exécution (énumération des disques impossible ou non disponible sur ce système) ;
/// - `2` : erreur d'usage (code de clap).
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    ExitCode::from(execute(
        args,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
        enumerate_disks,
    ))
}

/// Cœur de [`run`], avec les flux et l'énumération injectés pour les tests.
fn execute<I, T, O, E, F>(args: I, out: &mut O, err: &mut E, enumerate: F) -> u8
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
    O: Write,
    E: Write,
    F: FnOnce() -> Result<Vec<Disk>, EnumerateError>,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(erreur) => {
            // `print` choisit la sortie adaptée : aide et version sur stdout, erreurs sur stderr.
            // Un échec d'écriture sur ces flux ne doit pas masquer le code de sortie.
            let _ = erreur.print();
            return u8::try_from(erreur.exit_code()).unwrap_or(EXIT_USAGE);
        }
    };

    match cli.command {
        Command::List(args) => {
            let disks = match enumerate() {
                Ok(disks) => disks,
                Err(erreur) => {
                    let _ = writeln!(err, "bonk : erreur : {erreur}");
                    return EXIT_RUNTIME_ERROR;
                }
            };
            match list::run(&args, &disks, out) {
                Ok(()) => 0,
                Err(erreur) => {
                    let _ = writeln!(err, "bonk : erreur d'écriture de la sortie : {erreur}");
                    EXIT_RUNTIME_ERROR
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use bonk_hal::{Bus, DiskKind, Partition};
    use serde_json::Value;

    use super::*;

    fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
        Partition {
            path: path.to_owned(),
            label: None,
            fs_type: Some("ext4".to_owned()),
            size_bytes: 1 << 30,
            mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
            holders: vec![],
            is_system_volume: system,
            is_active_esp: false,
        }
    }

    fn disk(id: &str, path: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
        Disk {
            id: id.to_owned(),
            path: path.to_owned(),
            model: "Modèle de test".to_owned(),
            serial: None,
            size_bytes: 16_008_609_792,
            bus,
            kind: DiskKind::Physical,
            removable_flag: false,
            fs_type: None,
            holders: vec![],
            partitions,
        }
    }

    fn sample() -> Vec<Disk> {
        vec![
            disk(
                "nvme-systeme",
                "/dev/nvme0n1",
                Bus::Nvme,
                vec![partition("/dev/nvme0n1p2", &["/"], true)],
            ),
            disk("usb-cle", "/dev/sdb", Bus::Usb, vec![]),
            disk("sata-libre", "/dev/sda", Bus::Sata, vec![]),
        ]
    }

    /// Lance la commande avec des disques fixes ; renvoie code, sortie standard, sortie d'erreur.
    fn lancer(args: &[&str], disks: Vec<Disk>) -> (u8, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            std::iter::once("bonk").chain(args.iter().copied()),
            &mut out,
            &mut err,
            || Ok(disks),
        );
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    #[test]
    fn liste_par_defaut_ne_montre_que_les_disques_proposes() {
        let (code, out, err) = lancer(&["list"], sample());
        assert_eq!((code, err.as_str()), (0, ""));
        assert!(out.contains("usb-cle"), "{out}");
        assert!(out.contains("proposé"), "{out}");
        assert!(!out.contains("nvme-systeme"), "{out}");
        assert!(!out.contains("sata-libre"), "{out}");
    }

    #[test]
    fn all_montre_les_ecartes_avec_la_raison() {
        let (code, out, _) = lancer(&["list", "--all"], sample());
        assert_eq!(code, 0);
        assert!(
            out.contains("interdit : volume système (/dev/nvme0n1p2)"),
            "{out}"
        );
        assert!(out.contains("/dev/nvme0n1p2 monté sur /"), "{out}");
        assert!(out.contains("non affiché (bus interne"), "{out}");
    }

    #[test]
    fn show_internal_propose_le_disque_interne_avec_avertissement() {
        let (_, out, _) = lancer(&["list", "--show-internal"], sample());
        assert!(out.contains("sata-libre"), "{out}");
        assert!(out.contains("confirmation renforcée"), "{out}");
        assert!(!out.contains("nvme-systeme"), "{out}");
    }

    #[test]
    fn aucun_disque_donne_un_message_et_le_code_zero() {
        let (code, out, _) = lancer(&["list"], vec![]);
        assert_eq!(code, 0);
        assert!(out.contains("Aucun disque proposé."), "{out}");
        assert!(out.contains("--all"), "{out}");
    }

    #[test]
    fn colonnes_alignees() {
        let (_, out, _) = lancer(&["list", "--all"], sample());
        let columns: Vec<usize> = out
            .lines()
            .filter_map(|l| l.find("/dev/").or_else(|| l.find("CHEMIN")))
            .collect();
        assert_eq!(columns.len(), 4, "{out}");
        assert!(
            columns.windows(2).all(|w| w[0] == w[1]),
            "{columns:?}\n{out}"
        );
    }

    #[test]
    fn json_est_valide_et_versionne() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["list", "--json", "--all"], sample());
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["schema_version"], 1);
        let disks = json["disks"].as_array().map_or(0, Vec::len);
        assert_eq!(disks, 3);
        assert_eq!(json["disks"][0]["id"], "nvme-systeme");
        assert_eq!(json["disks"][0]["eligibility"]["verdict"], "excluded");
        assert_eq!(json["disks"][1]["eligibility"]["verdict"], "offered");
        assert_eq!(json["disks"][2]["eligibility"]["verdict"], "hidden");
        Ok(())
    }

    #[test]
    fn json_par_defaut_ne_contient_que_les_disques_proposes() -> Result<(), serde_json::Error> {
        let (_, out, _) = lancer(&["list", "--json"], sample());
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["disks"].as_array().map_or(0, Vec::len), 1);
        assert_eq!(json["disks"][0]["id"], "usb-cle");
        Ok(())
    }

    #[test]
    fn json_sans_disque_reste_du_json_valide() -> Result<(), serde_json::Error> {
        let (code, out, _) = lancer(&["list", "--json"], vec![]);
        assert_eq!(code, 0);
        let json: Value = serde_json::from_str(&out)?;
        assert_eq!(json["disks"].as_array().map(Vec::len), Some(0));
        Ok(())
    }

    #[test]
    fn echec_de_l_enumeration_donne_le_code_un_et_rien_sur_stdout() {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(["bonk", "list"], &mut out, &mut err, || {
            Err(EnumerateError::Unsupported { os: "test" })
        });
        assert_eq!(code, EXIT_RUNTIME_ERROR);
        assert!(out.is_empty());
        let err = String::from_utf8_lossy(&err);
        assert!(err.starts_with("bonk : erreur : "), "{err}");
    }

    #[test]
    fn option_inconnue_donne_le_code_d_usage() {
        let (code, out, _) = lancer(&["list", "--efface-tout"], sample());
        assert_eq!(code, EXIT_USAGE);
        assert!(out.is_empty());
    }
}
