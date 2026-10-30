//! Point d'entrée unique de Bonk : la fenêtre sans commande, la ligne de commande sinon.
//! La ligne de commande reçoit la fonction qui ouvre la fenêtre : les deux crates ne se connaissent pas.

use std::process::ExitCode;

fn main() -> ExitCode {
    bonk_cli::run(std::env::args_os(), |view| {
        bonk_gui::run(bonk_gui::ViewOptions {
            show_internal: view.show_internal,
            allow_virtual: view.allow_virtual,
            show_all: view.show_all,
        })
    })
}
