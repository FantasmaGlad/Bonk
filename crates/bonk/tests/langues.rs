//! La fenêtre donne un titre français à chaque code d'erreur que connaît la ligne de commande.
#![cfg(feature = "gui")]

use std::process::Command;

#[test]
fn chaque_code_a_son_titre_francais() -> std::io::Result<()> {
    let output = Command::new(env!("CARGO_BIN_EXE_bonk"))
        .arg("explain")
        .output()?;
    let list = String::from_utf8_lossy(&output.stdout);
    let codes: Vec<&str> = list
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .collect();
    assert!(codes.len() > 30, "{list}");
    let missing: Vec<&&str> = codes
        .iter()
        .filter(|code| bonk_gui::french_title(code).is_none())
        .collect();
    assert!(missing.is_empty(), "sans titre : {missing:?}");
    Ok(())
}
