use super::*;

pub(super) fn vieillir_le_panneau(app: &mut App) {
    if let WriteState::Draft { opened, .. } = &mut app.write {
        *opened = Instant::now() - Duration::from_secs(5);
    }
}

pub(super) fn figer_le_decompte(app: &mut App) {
    if let WriteState::Draft { opened, .. } = &mut app.write {
        *opened = Instant::now() + Duration::from_secs(60);
    }
}

pub(super) fn app_avec_panneau() -> App {
    let mut app = loaded(sample());
    // Never the real binary: the launch fails at once, writing nothing.
    app.program = Some(PathBuf::from("/n/existe/pas"));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    app
}

#[test]
fn le_panneau_d_ecriture_ne_s_ouvre_que_sur_un_disque_propose() {
    let mut app = loaded(sample());
    let _ = app.update(Message::ToggleAll(true));
    let _ = app.update(Message::Select("nvme-systeme".to_owned()));
    assert!(
        matches!(app.write, WriteState::Idle),
        "un disque interdit n'a pas de panneau"
    );
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert!(matches!(&app.write, WriteState::Draft { disk_id, .. } if disk_id == "usb-cle"));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert!(
        matches!(app.write, WriteState::Idle),
        "désélectionner referme le panneau"
    );
    // An internal disk is offered, so writable, only with the option.
    let _ = app.update(Message::Select("sata-libre".to_owned()));
    assert!(matches!(app.write, WriteState::Idle));
    let _ = app.update(Message::ToggleInternal(true));
    assert!(matches!(&app.write, WriteState::Draft { disk_id, .. } if disk_id == "sata-libre"));
    // Removing the option closes the panel: the disk is no longer offered.
    let _ = app.update(Message::ToggleInternal(false));
    assert!(matches!(app.write, WriteState::Idle));
}

#[test]
fn l_ecriture_exige_une_image_et_trois_secondes() {
    let mut app = app_avec_panneau();
    figer_le_decompte(&mut app);
    // Immediately: refused, even with an image.
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    let _ = app.update(Message::StartWrite);
    assert!(
        matches!(app.write, WriteState::Draft { .. }),
        "le délai de réflexion n'est pas écoulé"
    );
    vieillir_le_panneau(&mut app);
    // Delay over but empty field: refused.
    let _ = app.update(Message::ImageInput("   ".to_owned()));
    let _ = app.update(Message::StartWrite);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // Image entered and delay over: the write starts.
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    let _ = app.update(Message::StartWrite);
    assert!(matches!(&app.write, WriteState::Running { disk_id, .. } if disk_id == "usb-cle"));
}

#[test]
fn un_disque_devenu_interdit_n_est_pas_ecrit() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    vieillir_le_panneau(&mut app);
    // The key was unplugged meanwhile: the list no longer has it.
    let _ = app.update(Message::Loaded(Ok(vec![])));
    let _ = app.update(Message::StartWrite);
    assert!(matches!(app.write, WriteState::Idle));
}

pub(super) fn app_sur_disque_interne() -> App {
    let mut app = App {
        model: Model {
            show_internal: true,
            system_dark: Some(false),
            ..Model::default()
        },
        program: Some(PathBuf::from("/bonk-programme-inexistant")),
    };
    let _ = app.update(Message::Loaded(Ok(vec![disk(
        "sata-x",
        "/dev/sda",
        "Interne",
        Bus::Sata,
        vec![],
    )])));
    let _ = app.update(Message::Select("sata-x".to_owned()));
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    vieillir_le_panneau(&mut app);
    app
}

#[test]
fn un_disque_interne_demande_confirmation_avant_de_lancer() {
    let mut app = app_sur_disque_interne();
    let _ = app.update(Message::StartWrite);
    assert!(app.confirming_internal);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // The keyboard does not confirm: Enter does nothing, Escape gives up.
    let _ = app.update(Message::Nav(Nav::Activate));
    assert!(matches!(app.write, WriteState::Draft { .. }));
    let _ = app.update(Message::Nav(Nav::Escape));
    assert!(!app.confirming_internal);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    let _ = app.update(Message::StartWrite);
    let _ = app.update(Message::ConfirmInternal);
    assert!(!app.confirming_internal);
    assert!(matches!(app.write, WriteState::Running { .. }));
}

#[test]
fn la_demande_de_confirmation_previent_et_propose_les_deux_choix() {
    let mut app = app_sur_disque_interne();
    assert!(
        Simulator::new(app.view())
            .find("ATTENTION : DISQUE INTERNE")
            .is_err()
    );
    let _ = app.update(Message::StartWrite);
    let mut simulateur = Simulator::new(app.view());
    assert!(simulateur.find("ATTENTION : DISQUE INTERNE").is_ok());
    assert!(simulateur.find("Renoncer").is_ok());
    assert!(simulateur.find("Effacer ce disque interne").is_ok());
}

#[test]
fn un_disque_amovible_ne_demande_pas_de_confirmation_supplementaire() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    vieillir_le_panneau(&mut app);
    let _ = app.update(Message::StartWrite);
    assert!(!app.confirming_internal);
    assert!(matches!(app.write, WriteState::Running { .. }));
}

#[test]
fn progression_puis_resultat() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    vieillir_le_panneau(&mut app);
    let _ = app.update(Message::StartWrite);
    let _ = app.update(Message::Write(transfer::Event::Started(4242)));
    let _ = app.update(Message::Write(transfer::Event::Progress {
        phase: Phase::Write,
        done: 1_000_000,
        total: Some(4_000_000),
        rate: 2e6,
    }));
    assert!(matches!(
        &app.write,
        WriteState::Running {
            pid: Some(4242),
            done: 1_000_000,
            ..
        }
    ));
    assert!(contient(&app, "Écriture : 1,0 Mo sur 4,0 Mo  (2,0 Mo/s)"));
    assert!(contient(&app, "Annuler"));
    // During a write, deselecting the disk hides neither the progress nor the cancel.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert!(app.selected.is_none());
    assert!(matches!(app.write, WriteState::Running { .. }));
    assert!(contient(&app, "Annuler"));
    let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Done {
        bytes: 4_000_000,
        verified: true,
        gpt_repaired: true,
        secure_boot: None,
        bios_boot: false,
    })));
    assert!(matches!(&app.write, WriteState::Finished { .. }));
    assert!(contient(
        &app,
        "ÉCRITURE TERMINÉE : 4,0 Mo écrits, relus et vérifiés, GPT de secours réparé."
    ));
    let _ = app.update(Message::DismissWrite);
    // Closed: the disk is no longer selected, nothing opens; selecting it again opens a fresh panel.
    assert!(matches!(app.write, WriteState::Idle));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert!(matches!(app.write, WriteState::Draft { .. }));
}

#[test]
fn un_echec_s_affiche_avec_son_code_et_sa_commande_d_aide() {
    let mut app = app_avec_panneau();
    app.write = WriteState::Running {
        disk_id: "usb-cle".into(),
        pid: None,
        phase: Phase::Write,
        done: 0,
        total: None,
        rate: 0.0,
    };
    let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Failed {
        code: Some("E4004".into()),
        message: "ouverture du disque refusée".into(),
    })));
    assert!(contient(
        &app,
        "ÉCHEC : Ouverture du disque refusée (ouverture du disque refusée) [E4004] : voir « bonk explain E4004 »"
    ));
}

#[test]
fn le_panneau_montre_le_recapitulatif_et_le_decompte() {
    let app = app_avec_panneau();
    assert!(contient(&app, "ÉCRIRE UNE IMAGE SUR CE DISQUE"));
    assert!(contient(
        &app,
        "Toutes les données de ce disque seront effacées. Cette action est irréversible."
    ));
    // The recap names the image, then the disk it erases: no second full description of the disk.
    assert!(contient(&app, "Disque effacé : Cle de test, 16,0 Go"));
    assert!(contient(&app, "Image : aucune choisie pour l'instant"));
    assert!(!contient(&app, "1 partition(s)"));
    // Frozen clock: an opening time in the future gives zero elapsed time, however slow the rendering (a slow
    // runner already passed 3 s before the first check).
    let mut figee = app_avec_panneau();
    figer_le_decompte(&mut figee);
    assert!(contient(&figee, "Écrire et effacer ce disque (dans 3 s)"));
    let mut vieux = app_avec_panneau();
    vieillir_le_panneau(&mut vieux);
    assert!(contient(
        &vieux,
        "Écrire et effacer ce disque (choisir une image)"
    ));
    let _ = vieux.update(Message::ImageInput("/tmp/x.img".to_owned()));
    assert!(contient(&vieux, "Écrire et effacer ce disque"));
}

#[test]
fn le_decompte_diminue_avec_le_temps_ecoule() {
    assert_eq!(secondes_restantes(Duration::ZERO), 3);
    assert_eq!(secondes_restantes(Duration::from_millis(1)), 3);
    assert_eq!(secondes_restantes(Duration::from_secs(1)), 2);
    assert_eq!(secondes_restantes(Duration::from_millis(1500)), 2);
    assert_eq!(secondes_restantes(Duration::from_millis(2999)), 1);
    // Once the delay is over, the button shows no countdown.
    assert_eq!(secondes_restantes(Duration::from_secs(10)), 0);
}

#[test]
fn la_commande_equivalente_inclut_l_ecriture_saisie() {
    let mut app = app_avec_panneau();
    assert_eq!(app.equivalents().len(), 2);
    let _ = app.update(Message::ImageInput("/home/u/Mes images/a.iso".to_owned()));
    assert_eq!(
        app.equivalents().last().map(String::as_str),
        Some("bonk write usb-cle '/home/u/Mes images/a.iso' --yes")
    );
}

#[test]
fn le_chemin_saisi_n_est_jamais_journalise() {
    let event = ui_event(&Message::ImageInput("/home/alice/secret.img".to_owned()));
    // The log only knows the character count, never the path.
    assert_eq!(event, Some(("type_image_path", "22 caractères".to_owned())));
}

/// The image picked in the system picker fills the field, like typing; closing the picker without a choice
/// leaves the field alone.
#[test]
fn l_image_choisie_remplit_le_champ() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::BrowseImage);
    assert_eq!(app.focus, Some(Focus::Browse));
    let _ = app.update(Message::ImagePicked(Some(PathBuf::from("/tmp/debian.iso"))));
    assert!(matches!(&app.write, WriteState::Draft { image, .. } if image == "/tmp/debian.iso"));
    assert!(
        app.equivalents()
            .iter()
            .any(|c| c.contains("/tmp/debian.iso"))
    );
    let _ = app.update(Message::ImagePicked(None));
    assert!(matches!(&app.write, WriteState::Draft { image, .. } if image == "/tmp/debian.iso"));
}

#[test]
fn le_chemin_choisi_n_est_jamais_journalise() {
    let choisi = Message::ImagePicked(Some(PathBuf::from("/home/alice/secret.iso")));
    assert_eq!(
        ui_event(&choisi),
        Some(("image_picked", "choisie".to_owned()))
    );
    let aucune = Message::ImagePicked(None);
    assert_eq!(
        ui_event(&aucune),
        Some(("image_picked", "aucune".to_owned()))
    );
    assert_eq!(
        ui_event(&Message::BrowseImage),
        Some(("click_browse", String::new()))
    );
}

#[test]
fn le_clavier_parcourt_le_panneau_et_l_action_respecte_le_delai() {
    let mut app = app_avec_panneau();
    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    let _ = app.update(Message::ToggleAdvanced);
    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Details));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Menu));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Advanced));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Image));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Browse));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Hash));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Tor));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Raw));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Mbr));
    nav(&mut app, Nav::Next);
    assert_eq!(app.focus, Some(Focus::Action));
    // Space on the button before the delay: no effect.
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    app.focus = Some(Focus::Action);
    nav(&mut app, Nav::Activate);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    vieillir_le_panneau(&mut app);
    nav(&mut app, Nav::Activate);
    assert!(matches!(app.write, WriteState::Running { .. }));
    // The arrows skip the panel: Down does not stop on the field or the button.
    let mut autre = app_avec_panneau();
    let _ = autre.update(Message::ToggleAll(true));
    autre.focus = Some(Focus::Disk("usb-cle".to_owned()));
    nav(&mut autre, Nav::Down);
    assert_eq!(autre.focus, Some(Focus::Disk("sata-libre".to_owned())));
}

#[test]
fn les_touches_prises_par_le_champ_ne_sont_pas_des_commandes() {
    // The subscription filter only passes keys the widgets ignored: here, the mapping itself, unchanged.
    assert!(matches!(
        map_key(
            &Key::Named(Named::Space),
            keyboard::Modifiers::empty(),
            false
        ),
        Some(Message::Nav(Nav::Activate))
    ));
}
