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

#[test]
fn list_aide_decrit_les_options() -> Resultat {
    let sortie = bonk().args(["list", "--help"]).output()?;
    assert!(sortie.status.success());
    let texte = String::from_utf8(sortie.stdout)?;
    for option in ["--json", "--all", "--show-internal", "--allow-virtual"] {
        assert!(
            texte.contains(option),
            "{option} absent de l'aide :\n{texte}"
        );
    }
    Ok(())
}

/// Sur la machine qui exécute le test, `bonk list --json` doit soit réussir avec un JSON versionné,
/// soit échouer proprement (code 1, message sur stderr, rien sur stdout) : jamais un autre état.
/// Le contenu dépend de la machine et n'est pas vérifié ici ; les règles le sont par les tests des
/// crates `bonk-hal` et `bonk-cli`.
#[test]
fn list_json_reussit_ou_echoue_proprement() -> Resultat {
    let sortie = bonk().args(["list", "--json", "--all"]).output()?;
    match sortie.status.code() {
        Some(0) => {
            let texte = String::from_utf8(sortie.stdout)?;
            assert!(texte.trim_start().starts_with('{'), "{texte}");
            assert!(texte.contains("\"schema_version\": 1"), "{texte}");
            assert!(sortie.stderr.is_empty());
        }
        Some(1) => {
            assert!(sortie.stdout.is_empty());
            let erreur = String::from_utf8(sortie.stderr)?;
            assert!(erreur.starts_with("bonk : erreur : "), "{erreur}");
        }
        autre => panic!("code de sortie inattendu : {autre:?}"),
    }
    Ok(())
}

/// Hors Linux, l'énumération n'existe pas encore : échec propre et message explicite.
#[cfg(not(target_os = "linux"))]
#[test]
fn list_hors_linux_echoue_avec_un_message_clair() -> Resultat {
    let sortie = bonk().arg("list").output()?;
    assert_eq!(sortie.status.code(), Some(1));
    let erreur = String::from_utf8(sortie.stderr)?;
    assert!(erreur.contains("pas encore disponible"), "{erreur}");
    Ok(())
}
