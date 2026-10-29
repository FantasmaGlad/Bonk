//! Point d'entrée unique de Bonk : l'interface graphique sans argument (à partir du jalon J1),
//! la ligne de commande sinon. Pour l'instant, seule la ligne de commande existe.

use std::process::ExitCode;

fn main() -> ExitCode {
    bonk_cli::run(std::env::args_os())
}
