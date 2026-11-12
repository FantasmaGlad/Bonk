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

/// The main page carries the disks alone; language, theme and expert boxes live on the settings page.
#[test]
fn les_reglages_quittent_la_page_principale() {
    let mut app = loaded(sample());
    for absent in ["Actualiser", "English", "Disques internes non système"] {
        assert!(!contient(&app, absent), "{absent}");
    }
    assert!(contient(&app, "Paramètres"));
    let _ = app.update(Message::Settings(bonk_ui::settings::SettingsMessage::Open));
    for present in [
        "Langue",
        "English",
        "Thème",
        "Sombre",
        "Disques internes non système",
        "Bonk redémarre aussitôt pour l'appliquer.",
    ] {
        assert!(contient(&app, present), "{present}");
    }
    assert!(!contient(&app, "Cle de test"));
}

#[test]
fn rien_n_est_dessine_avant_que_le_systeme_dise_clair_ou_sombre() {
    let mut app = loaded(sample());
    app.system_dark = None;
    assert!(!contient(&app, "Cle de test"));
    let _ = app.update(Message::Settings(
        bonk_ui::settings::SettingsMessage::SystemDark(false),
    ));
    assert!(contient(&app, "Cle de test"));
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
    // Selected: the card stays readable, the technical lines wait behind their button.
    let serie = "Série : 4C530001   Nature : physique   Drapeau amovible du système : oui";
    assert!(contient(&app, "16,0 Go   CLE   USB"));
    assert!(!contient(&app, serie));
    let mut simulateur = Simulator::new(app.view());
    simulateur.click("Détails techniques")?;
    for message in simulateur.into_messages() {
        let _ = app.update(message);
    }
    assert!(contient(&app, serie));
    assert!(contient(&app, "Masquer les détails techniques"));
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
    captures_fenetre(&dossier)?;
    captures_ecriture(&dossier)?;
    captures_coffre(&dossier)?;
    capture(&avec_une_image(), 640.0, &dossier, "fenetre-bibliotheque")
}

type Capture = Result<(), Box<dyn std::error::Error>>;

/// In the theme chosen now: [`crate::theme::choose`] is global to the process.
fn capture(app: &App, height: f32, dossier: &std::path::Path, nom: &str) -> Capture {
    capture_a(app, Size::new(960.0, height), dossier, nom)
}

fn capture_a(app: &App, size: Size, dossier: &std::path::Path, nom: &str) -> Capture {
    Simulator::with_size(iced::Settings::default(), size, app.view())
        .snapshot(&crate::theme::bonk())?
        .matches_image(dossier.join(nom))?;
    Ok(())
}

fn captures_fenetre(dossier: &std::path::Path) -> Capture {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    capture(&app, 640.0, dossier, "fenetre-cle")?;
    let _ = app.update(Message::Settings(bonk_ui::settings::SettingsMessage::Open));
    capture(&app, 640.0, dossier, "fenetre-parametres")?;
    // The other tests read texts, never colours: the dark capture does not disturb them.
    crate::theme::choose(true);
    let sombre = capture(&app, 640.0, dossier, "fenetre-parametres-sombre");
    crate::theme::choose(false);
    sombre?;
    app.focus = Some(Focus::Virtual);
    capture(&app, 640.0, dossier, "fenetre-focus-case")?;
    let _ = app.update(Message::ToggleAll(true));
    let _ = app.update(Message::ToggleInternal(true));
    let _ = app.update(Message::Settings(bonk_ui::settings::SettingsMessage::Close));
    capture(&app, 640.0, dossier, "fenetre-tout")?;
    // Keyboard focus on the second card, detail closed.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    nav(&mut app, Nav::Escape);
    nav(&mut app, Nav::Previous);
    nav(&mut app, Nav::Previous);
    capture(&app, 640.0, dossier, "fenetre-focus")?;
    let mut erreur = App::default();
    let _ = erreur.update(Message::Loaded(Err("base udev introuvable".to_owned())));
    capture(&erreur, 640.0, dossier, "fenetre-erreur")
}

/// Write panel: an image entered, then progress.
fn captures_ecriture(dossier: &std::path::Path) -> Capture {
    let mut ecriture = loaded(sample());
    let _ = ecriture.update(Message::Select("usb-cle".to_owned()));
    let _ = ecriture.update(Message::ImageInput(
        "/home/alice/images/debian-12.iso".to_owned(),
    ));
    capture_a(
        &ecriture,
        Size::new(640.0, 900.0),
        dossier,
        "fenetre-ecriture-etroite",
    )?;
    vieillir_le_panneau(&mut ecriture);
    ecriture.focus = Some(Focus::Action);
    capture(&ecriture, 760.0, dossier, "fenetre-ecriture")?;
    ecriture.program = Some(PathBuf::from("/n/existe/pas"));
    let _ = ecriture.update(Message::StartWrite);
    let _ = ecriture.update(Message::Write(transfer::Event::Progress {
        phase: Phase::Write,
        done: 1_300_000_000,
        total: Some(2_100_000_000),
        rate: 41e6,
    }));
    capture(&ecriture, 760.0, dossier, "fenetre-progression")
}

/// Vault panel: listed, then a disk without a vault, then the question before erasing.
fn captures_coffre(dossier: &std::path::Path) -> Capture {
    let mut coffre = loaded(sample());
    let _ = coffre.update(Message::Select("usb-cle".to_owned()));
    let images = vec![
        (
            "debian-live-13.1.0-amd64-standard.iso".to_owned(),
            1_552_000_000,
        ),
        ("archlinux-x86_64.iso".to_owned(), 1_391_000_000),
    ];
    let disk_id = || "usb-cle".to_owned();
    for (nom, etat) in [
        (
            "fenetre-coffre",
            VaultState::Ready {
                disk_id: disk_id(),
                images,
            },
        ),
        (
            "fenetre-coffre-absent",
            VaultState::NotVault { disk_id: disk_id() },
        ),
        (
            "fenetre-coffre-confirmer",
            VaultState::Confirming { disk_id: disk_id() },
        ),
    ] {
        coffre.vault = etat;
        capture(&coffre, 760.0, dossier, nom)?;
    }
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
    app.vault = VaultState::NotVault {
        disk_id: "ailleurs".to_owned(),
    };
    assert!(!contient(&app, "Préparer en coffre") && !contient(&app, "Prepare as a vault"));
}
