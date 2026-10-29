//! Point d'entrée unique de Bonk : la fenêtre sans commande, la ligne de commande sinon (CDC §1).
//! La ligne de commande reçoit la fonction qui ouvre la fenêtre : les deux crates ne se connaissent pas.

use std::process::ExitCode;

fn main() -> ExitCode {
    bonk_cli::run(std::env::args_os(), bonk_gui::run)
}
