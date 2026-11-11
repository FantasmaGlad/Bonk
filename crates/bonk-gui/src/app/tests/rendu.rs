use super::*;

pub(super) fn contient(app: &App, texte: &str) -> bool {
    Simulator::new(app.view()).find(texte).is_ok()
}

#[test]
fn la_fenetre_affiche_la_cle_et_pas_le_disque_systeme() {
    let app = loaded(sample());
    assert!(contient(&app, "Cle de test"));
    assert!(contient(&app, "PROPOSÉ"));
    assert!(!contient(&app, "Disque systeme"));
    // No write button: the window is read-only.
    assert!(!contient(&app, "Écrire"));
}

#[test]
fn la_fenetre_explique_pourquoi_un_disque_est_ecarte() {
    let mut app = loaded(sample());
    let _ = app.update(Message::ToggleAll(true));
    assert!(contient(&app, "INTERDIT"));
    assert!(contient(
        &app,
        "interdit : volume système (/dev/nvme0n1p2) ; /dev/nvme0n1p2 monté sur /"
    ));
}

#[test]
fn un_clic_sur_la_carte_ouvre_le_detail() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = loaded(sample());
    let mut simulateur = Simulator::new(app.view());
    simulateur.click("Cle de test")?;
    let messages: Vec<Message> = simulateur.into_messages().collect();
    assert!(
        matches!(messages.as_slice(), [Message::Select(id)] if id == "usb-cle"),
        "{messages:?}"
    );
    for message in messages {
        let _ = app.update(message);
    }
    assert!(contient(
        &app,
        "Série : 4C530001   Nature : physique   Drapeau amovible du système : oui"
    ));
    Ok(())
}

#[test]
fn un_echec_s_affiche_en_toutes_lettres() {
    let mut app = App::default();
    let _ = app.update(Message::Loaded(Err("base udev introuvable".to_owned())));
    assert!(contient(
        &app,
        "ERREUR : les disques n'ont pas pu être détectés."
    ));
    assert!(contient(&app, "base udev introuvable"));
}

/// Writes PNG captures of the window to the directory `BONK_GUI_CAPTURES` names, to look at. Without the
/// variable the test does nothing: no image is compared or versioned.
#[test]
fn captures_facultatives() -> Result<(), Box<dyn std::error::Error>> {
    let Some(dossier) = std::env::var_os("BONK_GUI_CAPTURES") else {
        return Ok(());
    };
    let dossier = std::path::PathBuf::from(dossier);
    let mut app = loaded(sample());
    let theme = app.theme();
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 640.0),
        app.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-cle"))?;
    let _ = app.update(Message::ToggleAll(true));
    let _ = app.update(Message::ToggleInternal(true));
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 640.0),
        app.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-tout"))?;
    // Keyboard focus on the second card, detail closed.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    nav(&mut app, Nav::Escape);
    nav(&mut app, Nav::Previous);
    nav(&mut app, Nav::Previous);
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 640.0),
        app.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-focus"))?;
    app.focus = Some(Focus::Virtual);
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 640.0),
        app.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-focus-case"))?;
    // Write panel: an image entered, then progress.
    let mut ecriture = loaded(sample());
    let _ = ecriture.update(Message::Select("usb-cle".to_owned()));
    let _ = ecriture.update(Message::ImageInput(
        "/home/alice/images/debian-12.iso".to_owned(),
    ));
    vieillir_le_panneau(&mut ecriture);
    ecriture.focus = Some(Focus::Action);
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 760.0),
        ecriture.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-ecriture"))?;
    ecriture.program = Some(PathBuf::from("/n/existe/pas"));
    let _ = ecriture.update(Message::StartWrite);
    let _ = ecriture.update(Message::Write(transfer::Event::Progress {
        phase: Phase::Write,
        done: 1_300_000_000,
        total: Some(2_100_000_000),
        rate: 41e6,
    }));
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 760.0),
        ecriture.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-progression"))?;
    // Vault panel: listed, then a disk without a vault, then the question before erasing.
    let mut coffre = loaded(sample());
    let _ = coffre.update(Message::Select("usb-cle".to_owned()));
    for (nom, etat) in [
        (
            "fenetre-coffre",
            VaultState::Ready {
                disk_id: "usb-cle".to_owned(),
                images: vec![
                    (
                        "debian-live-13.1.0-amd64-standard.iso".to_owned(),
                        1_552_000_000,
                    ),
                    ("archlinux-x86_64.iso".to_owned(), 1_391_000_000),
                ],
            },
        ),
        (
            "fenetre-coffre-absent",
            VaultState::NotVault {
                disk_id: "usb-cle".to_owned(),
            },
        ),
        (
            "fenetre-coffre-confirmer",
            VaultState::Confirming {
                disk_id: "usb-cle".to_owned(),
            },
        ),
    ] {
        coffre.vault = etat;
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 760.0),
            coffre.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join(nom))?;
    }
    let mut erreur = App::default();
    let _ = erreur.update(Message::Loaded(Err("base udev introuvable".to_owned())));
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(960.0, 640.0),
        erreur.view(),
    )
    .snapshot(&theme)?
    .matches_image(dossier.join("fenetre-erreur"))?;
    Ok(())
}

/// The vault panel shows under the card of its disk: the ISO images, then the question before erasing.
#[test]
fn le_coffre_s_affiche_sous_son_disque() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    app.vault = VaultState::Ready {
        disk_id: "usb-cle".to_owned(),
        images: vec![("archlinux-x86_64.iso".to_owned(), 1_391_000_000)],
    };
    assert!(contient(&app, "archlinux-x86_64.iso"));
    app.vault = VaultState::Confirming {
        disk_id: "usb-cle".to_owned(),
    };
    assert!(contient(&app, "Effacer et préparer") || contient(&app, "Erase and prepare"));
    // On another disk, nothing.
    app.vault = VaultState::NotVault {
        disk_id: "ailleurs".to_owned(),
    };
    assert!(!contient(&app, "Préparer en coffre") && !contient(&app, "Prepare as a vault"));
}
