//! Ligne de commande de Bonk : analyse des arguments, sortie texte et JSON, codes de sortie stables.
//!
//! Dans cette version, seules les options `--help` et `--version` existent ; les sous-commandes
//! arrivent avec les jalons suivants.

use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

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
struct Cli {}

/// Exécute la ligne de commande et renvoie le code de sortie du processus.
///
/// Codes de sortie : `0` en cas de succès, `--help` et `--version` compris ; `2` pour une erreur
/// d'usage. On conserve les codes de clap pour que ce contrat reste stable d'une version à l'autre.
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(_) => ExitCode::SUCCESS,
        Err(erreur) => {
            // `print` choisit la sortie adaptée : aide et version sur stdout, erreurs sur stderr.
            // Un échec d'écriture sur ces flux ne doit pas masquer le code de sortie.
            let _ = erreur.print();
            ExitCode::from(u8::try_from(erreur.exit_code()).unwrap_or(2))
        }
    }
}
