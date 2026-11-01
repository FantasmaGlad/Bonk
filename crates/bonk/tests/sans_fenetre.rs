//! Version en ligne de commande, construite sans la fonctionnalité `gui` : tout marche, sauf la fenêtre, qui
//! n'existe pas et le dit.
#![cfg(not(feature = "gui"))]

mod common;

use std::process::Command;

fn bonk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
}

#[test]
fn sans_commande_elle_dit_qu_elle_n_a_pas_de_fenetre() -> Result<(), Box<dyn std::error::Error>> {
    let sortie = bonk().env("DISPLAY", ":0").output()?;
    common::assert_code(&sortie, 1);
    let erreur = String::from_utf8_lossy(&sortie.stderr);
    assert!(erreur.contains("without a window"), "{erreur}");
    assert!(erreur.contains("E5003"), "{erreur}");
    Ok(())
}

#[test]
fn les_commandes_fonctionnent() -> Result<(), Box<dyn std::error::Error>> {
    common::assert_code(&bonk().args(["explain", "E2001"]).output()?, 0);
    common::assert_code(&bonk().args(["schema", "write"]).output()?, 0);
    Ok(())
}
