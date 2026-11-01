use bonk_hal::{Bus, DiskKind, Partition};
use iced::keyboard::{Key, key::Named};
use iced_test::Simulator;

use super::navigation::*;
use super::view::*;
use super::*;

fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
    Partition {
        path: path.to_owned(),
        label: Some("CLE".to_owned()),
        fs_type: Some("vfat".to_owned()),
        size_bytes: 15_000_000_000,
        mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
        holders: vec![],
        is_system_volume: system,
        is_active_esp: false,
    }
}

fn disk(id: &str, path: &str, model: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
    Disk {
        id: id.to_owned(),
        path: path.to_owned(),
        model: model.to_owned(),
        serial: Some("4C530001".to_owned()),
        size_bytes: 16_008_609_792,
        bus,
        kind: DiskKind::Physical,
        removable_flag: true,
        fs_type: None,
        holders: vec![],
        partitions,
    }
}

fn sample() -> Vec<Disk> {
    vec![
        disk(
            "nvme-systeme",
            "/dev/nvme0n1",
            "Disque systeme",
            Bus::Nvme,
            vec![partition("/dev/nvme0n1p2", &["/"], true)],
        ),
        disk(
            "usb-cle",
            "/dev/sdb",
            "Cle de test",
            Bus::Usb,
            vec![partition("/dev/sdb1", &["/run/media/alice/CLE"], false)],
        ),
        disk(
            "sata-libre",
            "/dev/sda",
            "Disque interne libre",
            Bus::Sata,
            vec![],
        ),
    ]
}

fn loaded(disks: Vec<Disk>) -> App {
    let mut app = App::default();
    let _ = app.update(Message::Loaded(Ok(disks)));
    app
}

fn visible_ids(app: &App) -> Vec<&str> {
    app.entries().iter().map(|(d, _)| d.id.as_str()).collect()
}

#[test]
fn par_defaut_seuls_les_disques_proposes_sont_affiches() {
    assert_eq!(visible_ids(&loaded(sample())), ["usb-cle"]);
}

#[test]
fn les_cases_changent_l_affichage_sans_nouvelle_enumeration() {
    let mut app = loaded(sample());
    let _ = app.update(Message::ToggleAll(true));
    assert_eq!(visible_ids(&app), ["nvme-systeme", "usb-cle", "sata-libre"]);
    let _ = app.update(Message::ToggleAll(false));
    let _ = app.update(Message::ToggleInternal(true));
    assert_eq!(visible_ids(&app), ["usb-cle", "sata-libre"]);
    assert!(!app.loading);
}

#[test]
fn un_disque_systeme_n_est_jamais_affiche_comme_propose() {
    let mut app = loaded(sample());
    for message in [
        Message::ToggleAll(true),
        Message::ToggleInternal(true),
        Message::ToggleVirtual(true),
    ] {
        let _ = app.update(message);
    }
    let entries = app.entries();
    let system = entries.iter().find(|(d, _)| d.id == "nvme-systeme");
    assert!(system.is_some_and(|(_, verdict)| !verdict.is_writable()));
}

#[test]
fn la_selection_bascule_et_disparait_avec_le_disque() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.selected, None);
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    // La clé est débranchée : la sélection tombe.
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert_eq!(app.selected, None);
}

#[test]
fn une_enumeration_a_la_fois() {
    let mut app = App::default();
    let _ = app.update(Message::Refresh);
    assert!(app.loading);
    // Le minuteur ne relance rien tant que la précédente n'est pas finie : l'état reste « en cours ».
    let _ = app.update(Message::Tick);
    assert!(app.loading);
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert!(!app.loading);
}

#[test]
fn un_echec_vide_la_liste_au_lieu_de_la_garder() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Err("base udev introuvable".to_owned())));
    assert!(app.disks.is_empty());
    assert_eq!(app.selected, None);
    assert_eq!(app.error.as_deref(), Some("base udev introuvable"));
    // Une énumération réussie efface l'erreur.
    let _ = app.update(Message::Loaded(Ok(sample())));
    assert_eq!(app.error, None);
}

fn nav(app: &mut App, nav: Nav) {
    let _ = app.update(Message::Nav(nav));
}

fn avec_tout_affiche() -> App {
    let mut app = loaded(sample());
    let _ = app.update(Message::ToggleAll(true));
    let _ = app.update(Message::ToggleInternal(true));
    app.focus = None;
    app
}

#[test]
fn tab_parcourt_les_commandes_puis_les_disques_et_reboucle() {
    let mut app = avec_tout_affiche();
    let mut vus = Vec::new();
    for _ in 0..9 {
        nav(&mut app, Nav::Next);
        vus.push(app.focus.clone());
    }
    assert_eq!(
        vus,
        [
            Some(Focus::Refresh),
            Some(Focus::Internal),
            Some(Focus::Virtual),
            Some(Focus::All),
            Some(Focus::Disk("nvme-systeme".to_owned())),
            Some(Focus::Disk("usb-cle".to_owned())),
            Some(Focus::Disk("sata-libre".to_owned())),
            Some(Focus::Copy),
            Some(Focus::Refresh),
        ]
    );
}

#[test]
fn maj_tab_remonte_et_commence_par_le_dernier() {
    let mut app = avec_tout_affiche();
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Copy));
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
}

#[test]
fn espace_a_le_meme_effet_qu_un_clic() {
    let mut app = loaded(sample());
    app.focus = Some(Focus::Internal);
    nav(&mut app, Nav::Activate);
    assert!(app.show_internal);
    nav(&mut app, Nav::Activate);
    assert!(!app.show_internal);

    app.focus = Some(Focus::All);
    nav(&mut app, Nav::Activate);
    assert!(app.show_all);

    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    nav(&mut app, Nav::Activate);
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    nav(&mut app, Nav::Activate);
    assert_eq!(app.selected, None);

    app.focus = Some(Focus::Refresh);
    nav(&mut app, Nav::Activate);
    assert!(app.loading);
}

#[test]
fn rien_n_est_active_sans_focus() {
    let mut app = loaded(sample());
    nav(&mut app, Nav::Activate);
    assert!(!app.show_all && !app.show_internal && app.selected.is_none() && !app.loading);
}

#[test]
fn les_fleches_se_deplacent_entre_les_disques() {
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Refresh);
    // Bas depuis une commande entre dans la liste.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
    nav(&mut app, Nav::Down);
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Pas de bouclage en bas ni en haut.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    nav(&mut app, Nav::Up);
    nav(&mut app, Nav::Up);
    nav(&mut app, Nav::Up);
    assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
}

#[test]
fn echap_referme_le_detail_puis_retire_le_focus() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    nav(&mut app, Nav::Escape);
    assert_eq!(app.selected, None);
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
    nav(&mut app, Nav::Escape);
    assert_eq!(app.focus, None);
}

#[test]
fn le_focus_quitte_un_disque_qui_disparait() {
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Disk("sata-libre".to_owned()));
    // La case « écartés » masque le disque interne : le focus ne reste pas sur un composant absent.
    let _ = app.update(Message::ToggleInternal(false));
    assert_eq!(app.focus, Some(Focus::Internal));
    app.focus = Some(Focus::Disk("sata-libre".to_owned()));
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert_eq!(app.focus, None);
    // Un échec de détection retire aussi les disques de l'ordre de Tab.
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Err("panne".to_owned())));
    assert_eq!(app.focus, None);
    nav(&mut app, Nav::Next);
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Copy));
}

#[test]
fn un_clic_donne_le_focus_au_composant() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
    let _ = app.update(Message::ToggleAll(true));
    assert_eq!(app.focus, Some(Focus::All));
    // Le minuteur ne touche pas au focus.
    let _ = app.update(Message::Tick);
    assert_eq!(app.focus, Some(Focus::All));
}

#[test]
fn touches_vers_actions() {
    use keyboard::Modifiers as M;
    let tab = Key::Named(Named::Tab);
    let nav_de = |key: &Key, mods: M, repeat: bool| match map_key(key, mods, repeat) {
        Some(Message::Nav(n)) => Some(n),
        _ => None,
    };
    assert_eq!(nav_de(&tab, M::empty(), false), Some(Nav::Next));
    assert_eq!(nav_de(&tab, M::SHIFT, false), Some(Nav::Previous));
    assert_eq!(
        nav_de(&Key::Named(Named::Space), M::empty(), false),
        Some(Nav::Activate)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::Enter), M::empty(), false),
        Some(Nav::Activate)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::ArrowDown), M::empty(), false),
        Some(Nav::Down)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::ArrowUp), M::empty(), false),
        Some(Nav::Up)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::Escape), M::empty(), false),
        Some(Nav::Escape)
    );
    // Raccourcis du système laissés tranquilles, répétition de touche ignorée pour activer.
    assert_eq!(nav_de(&tab, M::CTRL, false), None);
    assert_eq!(nav_de(&tab, M::ALT, false), None);
    assert_eq!(nav_de(&Key::Named(Named::Space), M::empty(), true), None);
    assert_eq!(nav_de(&tab, M::empty(), true), Some(Nav::Next));
    assert_eq!(nav_de(&Key::Character("a".into()), M::empty(), false), None);
}

#[test]
fn haut_depuis_le_bouton_de_copie_revient_au_dernier_disque() {
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Copy);
    nav(&mut app, Nav::Up);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Bas depuis le dernier disque n'entre pas dans le bouton : c'est le rôle de Tab.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Sans disque affiché, Haut depuis la copie ne fait rien.
    let mut vide = loaded(vec![]);
    vide.focus = Some(Focus::Copy);
    nav(&mut vide, Nav::Up);
    assert_eq!(vide.focus, Some(Focus::Copy));
}

#[test]
fn la_commande_equivalente_suit_la_vue() {
    let mut app = loaded(sample());
    assert_eq!(app.equivalents(), ["bonk list"]);
    let _ = app.update(Message::ToggleAll(true));
    let _ = app.update(Message::ToggleInternal(true));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(
        app.equivalents(),
        [
            "bonk list --show-internal --all",
            "bonk info usb-cle --show-internal"
        ]
    );
    assert!(contient(
        &app,
        "bonk list --show-internal --all\nbonk info usb-cle --show-internal"
    ));
    assert!(contient(&app, "Commande équivalente"));
}

#[test]
fn le_bouton_copier_prend_le_focus_et_garde_la_vue() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Copy);
    assert_eq!(app.focus, Some(Focus::Copy));
    assert_eq!(app.equivalents(), ["bonk list"]);
}

#[test]
fn evenements_de_l_interface_journalises() {
    assert_eq!(
        ui_event(&Message::Refresh),
        Some(("click_refresh", String::new()))
    );
    assert_eq!(
        ui_event(&Message::ToggleInternal(true)),
        Some(("toggle_internal", "true".to_owned()))
    );
    assert_eq!(
        ui_event(&Message::Select("usb-cle".to_owned())),
        Some(("select_disk", "usb-cle".to_owned()))
    );
    assert_eq!(
        ui_event(&Message::Nav(Nav::Next)),
        Some(("key", "Next".to_owned()))
    );
    assert_eq!(
        ui_event(&Message::Copy),
        Some(("click_copy", String::new()))
    );
    // Le minuteur et les résultats ont leur propre journal.
    assert_eq!(ui_event(&Message::Tick), None);
    assert_eq!(ui_event(&Message::Loaded(Ok(vec![]))), None);
}

fn vieillir_le_panneau(app: &mut App) {
    if let WriteState::Draft { opened, .. } = &mut app.write {
        *opened = Instant::now() - Duration::from_secs(5);
    }
}

fn figer_le_decompte(app: &mut App) {
    if let WriteState::Draft { opened, .. } = &mut app.write {
        *opened = Instant::now() + Duration::from_secs(60);
    }
}

fn app_avec_panneau() -> App {
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

fn app_sur_disque_interne() -> App {
    let mut app = App {
        show_internal: true,
        program: Some(PathBuf::from("/bonk-programme-inexistant")),
        ..App::default()
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
        "ÉCHEC : ouverture du disque refusée [E4004] : voir « bonk explain E4004 »"
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

fn contient(app: &App, texte: &str) -> bool {
    Simulator::new(app.view()).find(texte).is_ok()
}

#[test]
fn la_fenetre_affiche_la_cle_et_pas_le_disque_systeme() {
    let app = loaded(sample());
    assert!(contient(&app, "Cle de test"));
    assert!(contient(&app, "PROPOSÉ"));
    assert!(!contient(&app, "Disque systeme"));
    // Pas de bouton d'écriture : la fenêtre est en lecture seule.
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

/// Écrit des captures PNG de la fenêtre dans le dossier désigné par `BONK_GUI_CAPTURES`, pour les
/// regarder. Sans cette variable, le test ne fait rien : aucune image n'est comparée ni versionnée.
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
    // Focus clavier sur la deuxième carte, détail fermé.
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
    // Panneau d'écriture : saisie d'une image, puis progression.
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

fn en_cours_d_ecriture() -> App {
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

/// Débranché ou énumération en échec : le sous-processus continue, la progression et « Annuler » restent.
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

/// Le résultat reste à l'écran jusqu'à « Fermer », code d'erreur compris, même si le disque est parti.
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
        "ÉCHEC : clé débranchée [E4001] : voir « bonk explain E4001 »"
    ));
    assert!(matches!(app.write, WriteState::Finished { .. }));
    let _ = app.update(Message::Select("sata-libre".to_owned()));
    assert!(matches!(app.write, WriteState::Finished { .. }));
}

/// « Fermer » ne rouvre pas un panneau d'écriture sur le disque qu'on vient d'écrire.
#[test]
fn fermer_le_resultat_ne_rouvre_pas_le_panneau() {
    let mut app = en_cours_d_ecriture();
    let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Done {
        bytes: 2,
        verified: true,
        gpt_repaired: false,
        secure_boot: None,
    })));
    let _ = app.update(Message::DismissWrite);
    assert!(matches!(app.write, WriteState::Idle), "{:?}", app.write);
    assert!(app.selected.is_none());
}

/// Le journal de la fenêtre ne porte jamais le chemin de l'image, même dans la commande équivalente.
#[test]
fn le_journal_ne_porte_pas_le_chemin_de_l_image() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/home/alice/secret.iso".into()));
    assert!(app.equivalents().iter().any(|c| c.contains("secret.iso")));
    assert!(!app.logged_equivalents().contains("secret.iso"));
}

/// L'empreinte et le mode saisis passent dans la commande équivalente, comme dans ce qui sera lancé.
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

fn versions_du_catalogue() -> Vec<catalog::System> {
    vec![catalog::System {
        name: "Ubuntu".to_owned(),
        icon: None,
        choices: vec![
            catalog::Choice {
                label: "26.04.1 desktop LTS".to_owned(),
                end_of_life: false,
                source: catalog::ChoiceSource::Download {
                    url: "https://u/26.iso".to_owned(),
                    sha256: "a".repeat(64),
                },
            },
            catalog::Choice {
                label: "24.04.5 desktop LTS".to_owned(),
                end_of_life: false,
                source: catalog::ChoiceSource::Download {
                    url: "https://u/24.iso".to_owned(),
                    sha256: "b".repeat(64),
                },
            },
            catalog::Choice {
                label: "11".to_owned(),
                end_of_life: false,
                source: catalog::ChoiceSource::OfficialPage("https://p/".to_owned()),
            },
        ],
    }]
}

fn app_avec_menu() -> App {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::CatalogLoaded(Ok(versions_du_catalogue())));
    app
}

#[test]
fn le_menu_s_ouvre_nomme_les_versions_et_un_choix_regle_tout() {
    let mut app = app_avec_menu();
    assert!(contient(&app, "Choisir un système..."));
    assert!(!contient(&app, "Ubuntu 26.04.1 desktop LTS"));
    let _ = app.update(Message::ToggleMenu);
    assert!(contient(&app, "Ubuntu 26.04.1 desktop LTS"));
    assert!(contient(
        &app,
        "à télécharger sur https://p/, puis Autre image"
    ));

    if let WriteState::Draft { raw, mbr, .. } = &mut app.write {
        (*raw, *mbr) = (true, true);
    }
    let _ = app.update(Message::MenuChose(
        versions_du_catalogue()[0].choices[0].clone(),
    ));
    assert!(matches!(
        &app.write,
        WriteState::Draft { image, hash, raw: false, mbr: false, .. }
            if image == "https://u/26.iso" && hash.len() == 64
    ));
    assert!(!app.menu_open);
    assert_eq!(
        app.focus,
        Some(Focus::Action),
        "le geste suivant est d'écrire"
    );
    assert!(
        contient(&app, "Ubuntu 26.04.1 desktop LTS"),
        "le choix reste nommé"
    );
    assert!(contient(
        &app,
        "Lien officiel et empreinte du catalogue signé : rien à régler."
    ));
}

#[test]
fn au_clavier_les_fleches_parcourent_le_menu_et_entree_choisit() {
    let mut app = app_avec_menu();
    app.focus = Some(Focus::Menu);
    let _ = app.update(Message::Nav(Nav::Activate));
    assert!(app.menu_open);
    let _ = app.update(Message::Nav(Nav::Down));
    let _ = app.update(Message::Nav(Nav::Down));
    assert_eq!(
        app.menu_index, 1,
        "la page officielle n'est pas une ligne à écrire"
    );
    let _ = app.update(Message::Nav(Nav::Activate));
    assert!(matches!(&app.write, WriteState::Draft { image, .. } if image == "https://u/24.iso"));

    let _ = app.update(Message::ToggleMenu);
    let _ = app.update(Message::Nav(Nav::Escape));
    assert!(!app.menu_open);
    assert!(app.selected.is_some(), "Échap referme le menu d'abord");
}

#[test]
fn catalogue_indisponible_la_voie_manuelle_se_montre() {
    let mut app = app_avec_panneau();
    assert!(!contient(&app, "Choisir l'image..."));
    let _ = app.update(Message::CatalogLoaded(Err("expired".to_owned())));
    assert!(contient(&app, "Liste des systèmes indisponible"));
    assert!(contient(&app, "expired"));
    assert!(contient(&app, "Choisir l'image..."));
}

#[test]
fn les_options_se_replient_et_quittent_l_ordre_du_clavier() {
    let mut app = app_avec_menu();
    assert!(!app.focus_order().contains(&Focus::Hash));
    let _ = app.update(Message::ToggleAdvanced);
    assert!(app.focus_order().contains(&Focus::Hash));
    let _ = app.update(Message::ImagePicked(Some(PathBuf::from("/x.iso"))));
    assert!(app.advanced);
}

#[test]
fn un_seul_disque_propose_se_selectionne_une_fois() {
    let mut app = App {
        auto_select: true,
        ..App::default()
    };
    let _ = app.update(Message::Loaded(Ok(sample())));
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // Écrire reste un geste : rien n'est lancé, et une désélection tient malgré l'actualisation.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Ok(sample())));
    assert_eq!(app.selected, None);
    // Deux disques : aucun choix à la place de l'utilisateur.
    let mut deux = App {
        auto_select: true,
        allow_virtual: false,
        show_internal: true,
        ..App::default()
    };
    let _ = deux.update(Message::Loaded(Ok(sample())));
    assert_eq!(deux.selected, None);
}
