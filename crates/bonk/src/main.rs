//! Point d'entrée unique de Bonk : l'interface graphique sans argument, la ligne de commande sinon
//! (CDC §1). La fenêtre actuelle est une détection des disques en lecture seule.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = std::env::args_os();
    if args.nth(1).is_none() {
        return launch_gui();
    }
    bonk_cli::run(std::env::args_os())
}

fn launch_gui() -> ExitCode {
    match bonk_gui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(erreur) => {
            // Pas d'affichage (SSH, serveur) ou pas de rendu utilisable : la CLI reste disponible.
            eprintln!("bonk : erreur : impossible d'ouvrir l'interface graphique : {erreur}");
            eprintln!("Utilisez la ligne de commande : bonk --help");
            ExitCode::from(1)
        }
    }
}
