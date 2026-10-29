//! Tests de bout en bout du binaire `bonk`, lancé tel que Cargo le construit : options
//! `--version` et `--help`, codes de sortie d'usage.

use std::process::Command;

type Resultat = Result<(), Box<dyn std::error::Error>>;

fn bonk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
}

#[test]
fn version_affiche_le_numero_du_paquet() -> Resultat {
    let sortie = bonk().arg("--version").output()?;
    assert!(sortie.status.success());
    let texte = String::from_utf8(sortie.stdout)?;
    assert_eq!(texte.trim(), format!("bonk {}", env!("CARGO_PKG_VERSION")));
    Ok(())
}

#[test]
fn help_reussit_et_commence_par_la_description_de_l_outil() -> Resultat {
    let sortie = bonk().arg("--help").output()?;
    assert!(sortie.status.success());
    let texte = String::from_utf8(sortie.stdout)?;
    assert_eq!(
        texte.lines().next(),
        Some("Création de supports d'amorçage (clés USB et disques externes)")
    );
    Ok(())
}

#[test]
fn sans_argument_affiche_l_aide_avec_le_code_d_usage() -> Resultat {
    let sortie = bonk().output()?;
    assert_eq!(sortie.status.code(), Some(2));
    // L'aide d'un appel sans argument part sur stderr : un binaire muet ne doit pas passer.
    let texte = String::from_utf8(sortie.stderr)?;
    assert_eq!(
        texte.lines().next(),
        Some("Création de supports d'amorçage (clés USB et disques externes)")
    );
    Ok(())
}

#[test]
fn option_inconnue_est_une_erreur_d_usage() -> Resultat {
    let sortie = bonk().arg("--inconnue").output()?;
    assert_eq!(sortie.status.code(), Some(2));
    Ok(())
}
