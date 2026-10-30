//! Disques internes : `--force` exigé, jamais suffisant pour un disque système.

use super::*;

#[test]
fn disque_interne_refuse_avec_yes_seul() {
    let source = TempSource::new("interne-yes", 4096);
    let scenario = with_internal();
    let (code, _, err) =
        scenario.run(&["write", "sata-x", &source.arg(), "--yes", "--show-internal"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains(CONFIRMATION_REQUIRED), "{err}");
    assert!(err.contains("--force"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn disque_interne_ecrit_avec_force_sans_yes_ni_question() {
    let source = TempSource::new("interne-force", 4096);
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&[
        "write",
        "sata-x",
        &source.arg(),
        "--show-internal",
        "--force",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(scenario.log().iter().any(|l| l == "unmount:sata-x"));
}

#[test]
fn force_tient_lieu_de_yes_sur_une_cle() {
    let source = TempSource::new("force-cle", 4096);
    let (code, _, err) = Scenario::new().run(&["write", "usb-a", &source.arg(), "--force"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn force_n_ouvre_ni_un_disque_interne_cache_ni_un_disque_systeme() {
    let source = TempSource::new("force-interdit", 4096);
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&["write", "sata-x", &source.arg(), "--force"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(err.contains("E2001"), "{err}");
    let mut scenario = Scenario::new();
    scenario.enumerations = vec![vec![usb("S1"), system_disk()]];
    let (code, _, err) = scenario.run(&[
        "write",
        "nvme-sys",
        &source.arg(),
        "--force",
        "--show-internal",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_reglage_n_autorise_pas_a_ecrire_sur_un_disque_interne() {
    let source = TempSource::new("interne-reglage", 4096);
    let mut scenario = with_internal();
    scenario.vars = vec![("BONK_VIEW_SHOW_INTERNAL".to_owned(), "1".to_owned())];
    let (code, _, err) = scenario.run(&["write", "sata-x", &source.arg(), "--force"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(err.contains("E2001"), "{err}");
    assert!(scenario.log().is_empty());
    // Le réglage garde son rôle d'affichage.
    let (code, out, _) = scenario.run(&["list"]);
    assert_eq!(code, 0);
    assert!(out.contains("sata-x"), "{out}");
}

#[test]
fn reparer_un_disque_interne_exige_aussi_force() {
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&["repair-gpt", "sata-x", "--yes", "--show-internal"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains(CONFIRMATION_REQUIRED), "{err}");
}
