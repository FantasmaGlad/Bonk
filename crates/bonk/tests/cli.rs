#![allow(clippy::too_many_lines, reason = "a scenario test reads in one piece")]

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
        Some("Create bootable media (USB drives and external disks)")
    );
    Ok(())
}

/// Without arguments Bonk opens the window. Without a display (SSH, server) it fails cleanly and points to
/// the command line, without a panic. The test removes the display variables: no window opens, even on a
/// desktop.
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
        erreur.starts_with("bonk: error: could not open the graphical interface"),
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

/// On the machine running the test, `bonk list --json` either succeeds with versioned JSON or fails cleanly
/// (code 1, message on stderr, nothing on stdout): never another state. The content depends on the machine;
/// the rules are tested in `bonk-hal` and `bonk-cli`.
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
            assert!(erreur.starts_with("bonk: error: "), "{erreur}");
        }
        autre => panic!("code de sortie inattendu : {autre:?}"),
    }
    Ok(())
}

/// The runner's disk carries the system: it must be listed and forbidden, never offered.
#[cfg(any(windows, target_os = "macos"))]
#[test]
fn le_disque_systeme_du_runner_est_interdit() -> Resultat {
    let sortie = bonk().args(["list", "--all", "--json"]).output()?;
    common::assert_code(&sortie, 0);
    let rapport: serde_json::Value = serde_json::from_slice(&sortie.stdout)?;
    let disques = rapport["disks"].as_array().ok_or("disks")?;
    let systeme: Vec<_> = disques
        .iter()
        .filter(|d| {
            d["partitions"]
                .as_array()
                .is_some_and(|parts| parts.iter().any(|p| p["is_system_volume"] == true))
        })
        .collect();
    assert_eq!(systeme.len(), 1, "{rapport}");
    assert_eq!(
        systeme[0]["eligibility"]["verdict"], "excluded",
        "{rapport}"
    );
    let sources = bonk().args(["debug", "sources", "--json"]).output()?;
    common::assert_code(&sources, 0);
    assert!(String::from_utf8(sources.stdout)?.contains(":facts"));
    Ok(())
}

/// No enumeration on this system: clean failure and explicit message.
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[test]
fn list_hors_linux_echoue_avec_un_message_clair() -> Resultat {
    let sortie = bonk().arg("list").output()?;
    common::assert_code(&sortie, 1);
    let erreur = String::from_utf8(sortie.stderr)?;
    assert!(erreur.contains("not available yet"), "{erreur}");
    Ok(())
}

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
            assert!(erreur.starts_with("bonk: error: "), "{erreur}");
        }
        autre => panic!("code de sortie inattendu : {autre:?}"),
    }
    Ok(())
}

/// The verbose log goes to stderr and never pollutes command output.
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

/// Under a virtual display (Xvfb) wgpu may panic ("incompatible window kind") instead of switching to
/// software rendering: Bonk then relaunches with `tiny-skia`, and the window stays open until `timeout` stops
/// it (code 124), without a panic. When wgpu succeeds (software driver present), only opening without a panic
/// is checked. Skipped outside CI if `xvfb-run` or `timeout` is missing; in CI their absence fails the test.
#[cfg(all(unix, not(target_os = "macos")))]
#[cfg(feature = "gui")]
#[test]
fn sous_affichage_virtuel_le_rendu_bascule_en_logiciel() -> Resultat {
    // Without an X server `bonk` does not even start and the test would say nothing of the fallback: skipped
    // outside CI, failed in CI (`common::decider`).
    if !common::affichage_virtuel_disponible() {
        return Ok(());
    }
    // An auth file of our own: without one, xvfb-run makes a temporary directory and exits 5 when removing it
    // races the display, whatever the command did.
    let autorisation = std::env::temp_dir().join(format!("bonk-xvfb-{}.auth", std::process::id()));
    let sortie = Command::new("xvfb-run")
        .arg("-f")
        .arg(&autorisation)
        .args(["-a", "-s", "-screen 0 1280x800x24", "timeout", "6"])
        .arg(env!("CARGO_BIN_EXE_bonk"))
        .arg("-v")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("XDG_SESSION_TYPE")
        .env_remove("ICED_BACKEND")
        .env_remove("BONK_RENDER_FALLBACK")
        .output()?;
    let _ = std::fs::remove_file(&autorisation);
    // Depending on the distribution `xvfb-run` sends the command's stderr to its stdout (`2>&1` in the Debian
    // script): the log is read from both streams.
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
    // The default panic message must not reach the user; the verbose log tells the fallback.
    assert!(!journal.contains("thread 'main' panicked"), "{journal}");
    // The fallback only happens if wgpu panics: on the maintainer's machine under Xvfb it does, not on the CI
    // runner, where a software driver (llvmpipe) lets wgpu succeed. When the window panics the fallback is
    // required; otherwise the test only ensures it opens without a panic.
    if journal.contains("window panicked") {
        assert!(
            journal.contains("restarting with the software renderer"),
            "{journal}"
        );
    }
    Ok(())
}

/// Capture then replay: what `list` sees here, `--fixture` gives back from the capture, without the serials
/// or the user name. Linux only (the only system with enumeration here); skipped if the enumeration fails
/// (container without udev).
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

    // No secret leaves in the capture: look for the serials read here.
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

    // Replaying must never be able to write to a real disk.
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
    let faux = dossier.join("faux.json");
    std::fs::write(&faux, "{}")?;
    let invalide = bonk().arg("--fixture").arg(&faux).arg("list").output()?;
    common::assert_code(&invalide, 2);
    std::fs::remove_dir_all(&dossier)?;
    Ok(())
}

/// Under `--fixture` the low level reads an image file, never a real disk, even named by its path.
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

/// The service writes to the disks it enumerates: under `--fixture` it would take the captured disks as real
/// and open the real `/dev/<name>` of the same name, which the filter never judged.
#[test]
fn fixture_refuse_le_service() -> Result<(), Box<dyn std::error::Error>> {
    let dossier = std::env::temp_dir().join(format!("bonk-fixture-serve-{}", std::process::id()));
    std::fs::create_dir_all(&dossier)?;
    let capture = dossier.join("capture.json");
    std::fs::write(
        &capture,
        r#"{"reads":[{"path":"/proc/swaps","kind":"file","ok":true,"bytes":3,"error":null,"micros":1,"content":"x\n"}]}"#,
    )?;
    let refus = bonk()
        .arg("--fixture")
        .arg(&capture)
        .arg("serve")
        .output()?;
    common::assert_code(&refus, 2);
    let erreur = String::from_utf8_lossy(&refus.stderr);
    assert!(erreur.contains("--fixture"), "{erreur}");
    assert!(erreur.trim_end().ends_with("[E5005]"), "{erreur}");
    std::fs::remove_dir_all(&dossier)?;
    Ok(())
}

/// `--no-gui`: a script never opens a window by accident, even with a display.
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

/// Colours forced by the environment: only the option can remove them then.
fn aide_coloree(options: &[&str]) -> Result<String, std::io::Error> {
    let sortie = Command::new(env!("CARGO_BIN_EXE_bonk"))
        .args(options)
        .arg("--help")
        .env("CLICOLOR_FORCE", "1")
        .env_remove("NO_COLOR")
        .output()?;
    assert!(sortie.status.success());
    Ok(String::from_utf8_lossy(&sortie.stdout).into_owned())
}

#[test]
fn no_color_retire_les_couleurs_de_l_aide() -> Result<(), std::io::Error> {
    assert!(aide_coloree(&[])?.contains('\u{1b}'));
    assert!(!aide_coloree(&["--no-color"])?.contains('\u{1b}'));
    Ok(())
}

#[test]
fn quiet_refuse_verbose_et_tait_les_avertissements() -> Result<(), std::io::Error> {
    let refus = Command::new(env!("CARGO_BIN_EXE_bonk"))
        .args(["--quiet", "-v", "explain"])
        .output()?;
    assert_eq!(refus.status.code(), Some(2));

    // An unreadable configuration is only a warning: the log says so, except with --quiet.
    let dossier = std::env::temp_dir().join(format!("bonk-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&dossier)?;
    let config = dossier.join("config.json");
    std::fs::write(&config, "{ cassé")?;
    let lancer = |options: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_bonk"))
            .args(options)
            .env("BONK_CONFIG", &config)
            .env_remove("BONK_LOG")
            .output()
    };
    let bavard = lancer(&["explain", "E1003"])?;
    assert!(String::from_utf8_lossy(&bavard.stderr).contains("configuration unreadable"));
    let quiet = lancer(&["--quiet", "explain", "E1003"])?;
    assert!(quiet.stderr.is_empty(), "{:?}", quiet.stderr);
    assert!(String::from_utf8_lossy(&quiet.stdout).contains("E1003"));
    std::fs::remove_dir_all(&dossier)?;
    Ok(())
}
