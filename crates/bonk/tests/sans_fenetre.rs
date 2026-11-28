#![cfg(not(feature = "gui"))]

mod common;

use std::process::Command;

fn bonk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
}

#[test]
fn sans_commande_elle_affiche_l_aide() -> Result<(), Box<dyn std::error::Error>> {
    let sortie = bonk().env("DISPLAY", ":0").output()?;
    common::assert_code(&sortie, 0);
    assert!(String::from_utf8(sortie.stdout)?.contains("Usage: bonk"));
    assert!(sortie.stderr.is_empty());
    Ok(())
}

#[test]
fn la_fenetre_demandee_dit_qu_elle_n_existe_pas() -> Result<(), Box<dyn std::error::Error>> {
    let sortie = bonk().env("DISPLAY", ":0").arg("gui").output()?;
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
