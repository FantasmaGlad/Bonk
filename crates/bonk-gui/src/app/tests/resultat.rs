use super::*;

pub(super) fn en_cours_d_ecriture() -> App {
    let mut app = app_avec_panneau();
    app.write = WriteState::Running {
        disk_id: "usb-cle".into(),
        pid: Some(7),
        phase: Phase::Write,
        done: 1,
        total: Some(2),
        rate: 0.0,
    };
    app
}

/// Unplugged, or enumeration failing: the child keeps running, progress and Cancel stay.
#[test]
fn l_ecriture_reste_visible_si_le_disque_quitte_la_liste() {
    let mut app = en_cours_d_ecriture();
    let _ = app.update(Message::Loaded(Ok(Vec::new())));
    assert!(contient(&app, "Annuler"));
    assert!(app.focus_order().contains(&Focus::Action));
    let mut app = en_cours_d_ecriture();
    let _ = app.update(Message::Loaded(Err("service absent".into())));
    assert!(contient(&app, "Annuler"));
}

/// The result stays on screen until Close, error code included, even if the disk is gone.
#[test]
fn le_resultat_reste_jusqu_a_la_fermeture() {
    let mut app = en_cours_d_ecriture();
    let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Failed {
        code: Some("E4001".into()),
        message: "clé débranchée".into(),
    })));
    let _ = app.update(Message::Loaded(Ok(Vec::new())));
    assert!(contient(
        &app,
        "ÉCHEC : Échec de l'écriture (clé débranchée) [E4001] : voir « bonk explain E4001 »"
    ));
    assert!(matches!(app.write, WriteState::Finished { .. }));
    let _ = app.update(Message::Select("sata-libre".to_owned()));
    assert!(matches!(app.write, WriteState::Finished { .. }));
}

#[test]
fn fermer_le_resultat_ne_rouvre_pas_le_panneau() {
    let mut app = en_cours_d_ecriture();
    let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Done {
        bytes: 2,
        verified: true,
        gpt_repaired: false,
        secure_boot: None,
        bios_boot: false,
    })));
    let _ = app.update(Message::DismissWrite);
    assert!(matches!(app.write, WriteState::Idle), "{:?}", app.write);
    assert!(app.selected.is_none());
}

/// The window log never holds the image path, even in the equivalent command.
#[test]
fn le_journal_ne_porte_pas_le_chemin_de_l_image() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/home/alice/secret.iso".into()));
    assert!(app.equivalents().iter().any(|c| c.contains("secret.iso")));
    assert!(!app.logged_equivalents().contains("secret.iso"));
}

/// The hash and mode entered go into the equivalent command, as into what will run.
#[test]
fn empreinte_et_mode_brut_arrivent_dans_la_commande() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("https://a.org/x.iso".into()));
    let _ = app.update(Message::HashInput("https://a.org/SHA256SUMS".into()));
    let _ = app.update(Message::ToggleRaw(true));
    let commandes = app.equivalents().join("\n");
    assert!(
        commandes.contains("--sha256-file https://a.org/SHA256SUMS --mode raw"),
        "{commandes}"
    );
}
