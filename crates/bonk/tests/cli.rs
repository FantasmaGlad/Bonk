//! Tests de bout en bout du binaire `bonk`, lancé tel que Cargo le construit : options
//! `--version` et `--help`, codes de sortie d'usage, `list` et lancement sans argument.

use std::process::Command;

mod common;

type Resultat = Result<(), Box<dyn std::error::Error>>;

fn bonk() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
}

#[test]
fn version_affiche_le_numero_du_paquet() -> Resultat {
    let sortie = bonk().arg("--version").output()?;
    common::assert_code(&sortie, 0);
    let texte = String::from_utf8(sortie.stdout)?;
    assert_eq!(texte.trim(), format!("bonk {}", env!("CARGO_PKG_VERSION")));
    Ok(())
}

#[test]
fn help_reussit_et_commence_par_la_description_de_l_outil() -> Resultat {
    let sortie = bonk().arg("--help").output()?;
    common::assert_code(&sortie, 0);
    let texte = String::from_utf8(sortie.stdout)?;
    assert_eq!(
        texte.lines().next(),
        Some("Création de supports d'amorçage (clés USB et disques externes)")
    );
    Ok(())
}

/// Sans argument, Bonk ouvre la fenêtre. Sans affichage graphique (SSH, serveur), il échoue
/// proprement et renvoie vers la ligne de commande, sans panique. Le test retire les variables
/// d'affichage : aucune fenêtre ne s'ouvre, même sur un poste de bureau.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn sans_argument_et_sans_affichage_renvoie_vers_la_ligne_de_commande() -> Resultat {
    let sortie = bonk()
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()?;
    common::assert_code(&sortie, 1);
    let erreur = String::from_utf8(sortie.stderr)?;
    assert!(
        erreur.starts_with("bonk : erreur : impossible d'ouvrir l'interface graphique"),
        "{erreur}"
    );
    assert!(erreur.contains("bonk --help"), "{erreur}");
    assert!(!erreur.contains("panicked"), "{erreur}");
    Ok(())
}

#[test]
fn option_inconnue_est_une_erreur_d_usage() -> Resultat {
    let sortie = bonk().arg("--inconnue").output()?;
    common::assert_code(&sortie, 2);
    Ok(())
}

#[test]
fn list_aide_decrit_les_options() -> Resultat {
    let sortie = bonk().args(["list", "--help"]).output()?;
    common::assert_code(&sortie, 0);
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
    common::assert_code(&sortie, 1);
    let erreur = String::from_utf8(sortie.stderr)?;
    assert!(erreur.contains("pas encore disponible"), "{erreur}");
    Ok(())
}

/// `debug sources` rend un rapport JSON versionné avec toutes les lectures, ou échoue proprement.
#[test]
fn debug_sources_json_reussit_ou_echoue_proprement() -> Resultat {
    let sortie = bonk().args(["debug", "sources", "--json"]).output()?;
    match sortie.status.code() {
        Some(0) => {
            let texte = String::from_utf8(sortie.stdout)?;
            assert!(texte.contains("\"schema_version\": 1"), "{texte}");
            assert!(texte.contains("\"reads\""), "{texte}");
        }
        Some(1) => {
            let erreur = String::from_utf8(sortie.stderr)?;
            assert!(erreur.starts_with("bonk : erreur : "), "{erreur}");
        }
        autre => panic!("code de sortie inattendu : {autre:?}"),
    }
    Ok(())
}

/// Le journal verbeux va sur la sortie d'erreur et ne pollue jamais la sortie des commandes.
#[test]
fn le_journal_va_sur_stderr_et_la_sortie_reste_du_json() -> Resultat {
    let sortie = bonk()
        .args(["-vv", "--log-format", "json", "diagnose"])
        .output()?;
    common::assert_code(&sortie, 0);
    let rapport = String::from_utf8(sortie.stdout)?;
    assert!(
        rapport.trim_start().starts_with('{') && rapport.contains("\"schema_version\""),
        "{rapport}"
    );
    let journal = String::from_utf8(sortie.stderr)?;
    let lignes: Vec<&str> = journal.lines().collect();
    assert!(!lignes.is_empty());
    // Une ligne par événement, chacune un objet JSON complet.
    for ligne in &lignes {
        assert!(
            ligne.starts_with("{\"component\"") && ligne.ends_with('}'),
            "{ligne}"
        );
        assert!(
            ligne.contains("\"session\"") && ligne.contains("\"ts\""),
            "{ligne}"
        );
    }
    Ok(())
}

/// Sous un affichage virtuel (Xvfb), wgpu peut paniquer (« incompatible window kind ») au lieu de
/// basculer sur le rendu logiciel : Bonk se relance alors en `tiny-skia`, et la fenêtre reste ouverte
/// jusqu'à l'arrêt par `timeout` (code 124), sans panique. Quand wgpu réussit (pilote logiciel
/// présent), seule l'ouverture sans panique est vérifiée. Ignoré hors CI si `xvfb-run` ou `timeout`
/// manque ; en CI, leur absence fait échouer le test.
#[cfg(all(unix, not(target_os = "macos")))]
#[cfg(feature = "gui")]
#[test]
fn sous_affichage_virtuel_le_rendu_bascule_en_logiciel() -> Resultat {
    // Sans serveur X, `bonk` ne se lance même pas et le test ne dirait rien du repli : saut hors CI,
    // échec en CI (`common::decider`).
    if !common::affichage_virtuel_disponible() {
        return Ok(());
    }
    let sortie = Command::new("xvfb-run")
        .args(["-a", "-s", "-screen 0 1280x800x24", "timeout", "6"])
        .arg(env!("CARGO_BIN_EXE_bonk"))
        .arg("-v")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("XDG_SESSION_TYPE")
        .env_remove("ICED_BACKEND")
        .env_remove("BONK_RENDER_FALLBACK")
        .output()?;
    // Selon la distribution, `xvfb-run` renvoie la sortie d'erreur de la commande sur sa sortie standard
    // (`2>&1` du script Debian) : le journal se lit donc dans les deux flux.
    let journal = format!(
        "{}{}",
        String::from_utf8_lossy(&sortie.stdout),
        String::from_utf8_lossy(&sortie.stderr)
    );
    assert_eq!(
        sortie.status.code(),
        Some(124),
        "statut : {:?}\n{journal}",
        sortie.status,
    );
    // Le message par défaut d'une panique (« thread 'main' panicked ») ne doit pas atteindre l'utilisateur ;
    // le journal verbeux, lui, raconte le repli.
    assert!(!journal.contains("thread 'main' panicked"), "{journal}");
    // Le repli n'a lieu que si wgpu panique : sur la machine du mainteneur c'est le cas sous Xvfb, pas sur
    // l'exécuteur de CI, où un pilote logiciel (llvmpipe) permet à wgpu de réussir. Quand la fenêtre
    // panique, le repli est donc exigé ; sinon le test garantit seulement qu'elle s'ouvre sans panique.
    if journal.contains("window panicked") {
        assert!(
            journal.contains("restarting with the software renderer"),
            "{journal}"
        );
    }
    Ok(())
}

/// Capture puis rejeu : ce que `list` voit sur cette machine, `--fixture` le redonne depuis la capture,
/// sans que les numéros de série ni le nom d'utilisateur y figurent. Linux seulement (seul système où
/// l'énumération existe) ; ignoré si l'énumération échoue ici (conteneur sans udev).
#[cfg(target_os = "linux")]
#[test]
fn une_capture_rejouee_donne_les_memes_verdicts() -> Resultat {
    let dossier = std::env::temp_dir().join(format!("bonk-capture-{}", std::process::id()));
    std::fs::create_dir_all(&dossier)?;
    let capture = dossier.join("capture.json");
    let sortie = bonk()
        .args(["diagnose", "--capture", "--output"])
        .arg(&capture)
        .output()?;
    assert_eq!(
        sortie.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&sortie.stderr)
    );
    let direct = bonk().args(["list", "--all", "--json"]).output()?;
    if direct.status.code() != Some(0) {
        eprintln!("énumération impossible ici : test ignoré");
        std::fs::remove_dir_all(&dossier)?;
        return Ok(());
    }
    let rejoue = bonk()
        .arg("--fixture")
        .arg(&capture)
        .args(["list", "--all", "--json"])
        .output()?;
    assert_eq!(
        rejoue.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&rejoue.stderr)
    );

    let resume =
        |octets: &[u8]| -> Result<Vec<(String, String, String, String)>, serde_json::Error> {
            let json: serde_json::Value = serde_json::from_slice(octets)?;
            Ok(json["disks"].as_array().map_or(vec![], |d| {
                d.iter()
                    .map(|x| {
                        (
                            x["path"].to_string(),
                            x["bus"].to_string(),
                            x["kind"].to_string(),
                            x["eligibility"]["verdict"].to_string(),
                        )
                    })
                    .collect()
            }))
        };
    let (a, b) = (resume(&direct.stdout)?, resume(&rejoue.stdout)?);
    assert!(!a.is_empty());
    assert_eq!(a, b, "la capture rejouée diffère de la lecture directe");

    // Aucun secret ne sort dans la capture : on cherche les numéros de série lus ici.
    let texte = std::fs::read_to_string(&capture)?;
    let direct_json: serde_json::Value = serde_json::from_slice(&direct.stdout)?;
    for disk in direct_json["disks"].as_array().into_iter().flatten() {
        if let Some(serie) = disk["serial"].as_str().filter(|s| s.len() >= 6) {
            assert!(!texte.contains(serie), "numéro de série dans la capture");
        }
    }
    if let Ok(user) = std::env::var("USER").map(|u| format!("/{u}/"))
        && user.len() > 4
    {
        assert!(!texte.contains(&user), "nom d'utilisateur dans la capture");
    }

    // Rejouer ne doit jamais pouvoir écrire sur un vrai disque.
    let id = direct_json["disks"][0]["id"]
        .as_str()
        .unwrap_or("x")
        .to_owned();
    let refus = bonk()
        .arg("--fixture")
        .arg(&capture)
        .args(["write", &id, "/dev/null", "--yes"])
        .output()?;
    common::assert_code(&refus, 2);
    assert!(
        String::from_utf8_lossy(&refus.stderr)
            .trim_end()
            .ends_with("[E5005]")
    );
    let sources = bonk()
        .arg("--fixture")
        .arg(&capture)
        .args(["debug", "sources"])
        .output()?;
    common::assert_code(&sources, 2);
    // Un fichier qui n'est pas une capture est refusé clairement.
    let faux = dossier.join("faux.json");
    std::fs::write(&faux, "{}")?;
    let invalide = bonk().arg("--fixture").arg(&faux).arg("list").output()?;
    common::assert_code(&invalide, 2);
    std::fs::remove_dir_all(&dossier)?;
    Ok(())
}

/// Sous `--fixture`, le bas niveau lit un fichier image, jamais un disque réel, même désigné par son chemin.
#[test]
fn fixture_refuse_la_lecture_brute_d_un_disque() -> Result<(), Box<dyn std::error::Error>> {
    let dossier = std::env::temp_dir().join(format!("bonk-fixture-brut-{}", std::process::id()));
    std::fs::create_dir_all(&dossier)?;
    let capture = dossier.join("capture.json");
    std::fs::write(
        &capture,
        r#"{"reads":[{"path":"/proc/swaps","kind":"file","ok":true,"bytes":3,"error":null,"micros":1,"content":"x\n"}]}"#,
    )?;
    let image = dossier.join("image.img");
    std::fs::write(&image, vec![0u8; 4096])?;
    for action in ["layout", "hexdump"] {
        for cible in ["/dev/sda", "usb-une-cle"] {
            let refus = bonk()
                .arg("--fixture")
                .arg(&capture)
                .args(["debug", action, cible])
                .output()?;
            assert_eq!(refus.status.code(), Some(2), "{action} {cible}");
            assert!(String::from_utf8_lossy(&refus.stderr).contains("--fixture"));
        }
        let permis = bonk()
            .arg("--fixture")
            .arg(&capture)
            .args(["debug", action])
            .arg(&image)
            .output()?;
        assert!(
            permis.status.success(),
            "{action} : {}",
            String::from_utf8_lossy(&permis.stderr)
        );
    }
    std::fs::remove_dir_all(&dossier)?;
    Ok(())
}

/// `--no-gui` : un script n'ouvre jamais de fenêtre par accident, même sous un affichage.
#[test]
fn no_gui_affiche_l_aide_et_n_ouvre_jamais_la_fenetre() -> Result<(), Box<dyn std::error::Error>> {
    let seul = bonk().env("DISPLAY", ":0").arg("--no-gui").output()?;
    common::assert_code(&seul, 2);
    assert!(String::from_utf8_lossy(&seul.stderr).contains("Usage"));
    let contradiction = bonk().args(["--no-gui", "gui"]).output()?;
    common::assert_code(&contradiction, 2);
    let commande = bonk().args(["--no-gui", "explain", "E2001"]).output()?;
    common::assert_code(&commande, 0);
    Ok(())
}
