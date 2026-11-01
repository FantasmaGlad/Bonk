//! Panneau d'écriture : ouverture, délai, confirmation, image.

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
    // Jamais le vrai binaire : le lancement échoue tout de suite, sans rien écrire.
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
    // Un disque interne n'est proposé (donc écrivable) qu'avec l'option.
    let _ = app.update(Message::Select("sata-libre".to_owned()));
    assert!(matches!(app.write, WriteState::Idle));
    let _ = app.update(Message::ToggleInternal(true));
    assert!(matches!(&app.write, WriteState::Draft { disk_id, .. } if disk_id == "sata-libre"));
    // Retirer l'option referme le panneau : le disque n'est plus proposé.
    let _ = app.update(Message::ToggleInternal(false));
    assert!(matches!(app.write, WriteState::Idle));
}

#[test]
fn l_ecriture_exige_une_image_et_trois_secondes() {
    let mut app = app_avec_panneau();
    figer_le_decompte(&mut app);
    // Immédiatement : refusé, même avec une image.
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    let _ = app.update(Message::StartWrite);
    assert!(
        matches!(app.write, WriteState::Draft { .. }),
        "le délai de réflexion n'est pas écoulé"
    );
    vieillir_le_panneau(&mut app);
    // Délai écoulé mais champ vide : refusé.
    let _ = app.update(Message::ImageInput("   ".to_owned()));
    let _ = app.update(Message::StartWrite);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // Image saisie et délai écoulé : l'écriture démarre.
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    let _ = app.update(Message::StartWrite);
    assert!(matches!(&app.write, WriteState::Running { disk_id, .. } if disk_id == "usb-cle"));
}

#[test]
fn un_disque_devenu_interdit_n_est_pas_ecrit() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    vieillir_le_panneau(&mut app);
    // La clé est débranchée entre-temps : la liste ne la contient plus.
    let _ = app.update(Message::Loaded(Ok(vec![])));
    let _ = app.update(Message::StartWrite);
    assert!(matches!(app.write, WriteState::Idle));
}

pub(super) fn app_sur_disque_interne() -> App {
    let mut app = App {
        model: Model {
            show_internal: true,
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
    // Le clavier ne confirme pas : Entrée ne fait rien, Échap renonce.
    let _ = app.update(Message::Nav(Nav::Activate));
    assert!(matches!(app.write, WriteState::Draft { .. }));
    let _ = app.update(Message::Nav(Nav::Escape));
    assert!(!app.confirming_internal);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // Confirmer lance l'écriture.
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
    // Pendant l'écriture, désélectionner le disque ne cache pas la progression ni l'annulation.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert!(app.selected.is_none());
    assert!(matches!(app.write, WriteState::Running { .. }));
    assert!(contient(&app, "Annuler"));
    let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Done {
        bytes: 4_000_000,
        verified: true,
        gpt_repaired: true,
        secure_boot: None,
    })));
    assert!(matches!(&app.write, WriteState::Finished { .. }));
    assert!(contient(
        &app,
        "ÉCRITURE TERMINÉE : 4,0 Mo écrits, relus et vérifiés, GPT de secours réparé."
    ));
    let _ = app.update(Message::DismissWrite);
    // Fermé : le disque n'est plus sélectionné, rien ne s'ouvre ; le resélectionner rouvre un panneau neuf.
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
    assert!(contient(
        &app,
        "Cle de test  série 4C530001  16,0 Go  1 partition(s)  étiquettes : CLE"
    ));
    // Horloge figée : un instant d'ouverture dans le futur donne un temps écoulé nul, quelle que soit la
    // lenteur du rendu (un exécuteur lent a déjà dépassé 3 s avant la première vérification).
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
    // Une fois le délai passé, le bouton n'affiche plus de décompte.
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
    // Seul le nombre de caractères est connu du journal, jamais le chemin.
    assert_eq!(event, Some(("type_image_path", "22 caractères".to_owned())));
}

/// L'image choisie dans le sélecteur du système remplit le champ, comme une saisie ; fermer le
/// sélecteur sans choisir ne touche pas à ce qui était saisi.
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
    // Espace sur le bouton avant le délai : sans effet.
    let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
    app.focus = Some(Focus::Action);
    nav(&mut app, Nav::Activate);
    assert!(matches!(app.write, WriteState::Draft { .. }));
    vieillir_le_panneau(&mut app);
    nav(&mut app, Nav::Activate);
    assert!(matches!(app.write, WriteState::Running { .. }));
    // Les flèches sautent le panneau : Bas ne s'arrête pas sur le champ ou le bouton.
    let mut autre = app_avec_panneau();
    let _ = autre.update(Message::ToggleAll(true));
    autre.focus = Some(Focus::Disk("usb-cle".to_owned()));
    nav(&mut autre, Nav::Down);
    assert_eq!(autre.focus, Some(Focus::Disk("sata-libre".to_owned())));
}

#[test]
fn les_touches_prises_par_le_champ_ne_sont_pas_des_commandes() {
    // Le filtre de l'abonnement ne transmet que les touches ignorées par les composants : ici, la
    // correspondance elle-même, qui reste inchangée.
    assert!(matches!(
        map_key(
            &Key::Named(Named::Space),
            keyboard::Modifiers::empty(),
            false
        ),
        Some(Message::Nav(Nav::Activate))
    ));
}
