use super::*;

pub(super) fn versions_du_catalogue() -> Vec<catalog::System> {
    vec![catalog::System {
        name: "Ubuntu".to_owned(),
        uses: vec!["office".to_owned(), "server".to_owned()],
        base: Some("ubuntu".to_owned()),
        icon: None,
        choices: vec![
            catalog::Choice {
                id: "ubuntu-26.04-desktop-x86_64".to_owned(),
                name: "Ubuntu".to_owned(),
                label: "26.04.1 desktop LTS".to_owned(),
                end_of_life: false,
                note: None,
                source: catalog::ChoiceSource::Download {
                    url: "https://u/26.iso".to_owned(),
                    sha256: "a".repeat(64),
                },
            },
            catalog::Choice {
                id: "ubuntu-24.04-desktop-x86_64".to_owned(),
                name: "Ubuntu".to_owned(),
                label: "24.04.5 desktop LTS".to_owned(),
                end_of_life: false,
                note: None,
                source: catalog::ChoiceSource::Download {
                    url: "https://u/24.iso".to_owned(),
                    sha256: "b".repeat(64),
                },
            },
            catalog::Choice {
                id: "ubuntu-11".to_owned(),
                name: "Ubuntu".to_owned(),
                label: "11".to_owned(),
                end_of_life: false,
                note: None,
                source: catalog::ChoiceSource::OfficialPage("https://p/".to_owned()),
            },
        ],
    }]
}

pub(super) fn etagere() -> catalog::Shelf {
    catalog::Shelf {
        systems: versions_du_catalogue(),
        terms: bonk_ui::terms::Terms::default(),
    }
}

pub(super) fn app_avec_menu() -> App {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::CatalogLoaded(Ok(etagere())));
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
        (*raw, *mbr) = (true, false);
    }
    let _ = app.update(Message::MenuChose(
        versions_du_catalogue()[0].choices[0].clone(),
    ));
    assert!(matches!(
        &app.write,
        WriteState::Draft { image, hash, raw: false, mbr: true, .. }
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
fn hors_ligne_le_catalogue_disparait_et_une_adresse_web_est_refusee() {
    let mut app = app_avec_menu();
    let _ = app.update(Message::Settings(
        bonk_ui::settings::SettingsMessage::Offline(true),
    ));
    let _ = app.update(Message::Settings(bonk_ui::settings::SettingsMessage::Close));
    assert!(!contient(&app, "Choisir un système..."));
    assert!(contient(
        &app,
        "Mode hors ligne : choisissez une image présente sur cet ordinateur."
    ));
    let _ = app.update(Message::ImageInput("https://u/26.iso".to_owned()));
    assert!(contient(
        &app,
        "Mode hors ligne : une adresse web n'est pas téléchargée."
    ));
}

#[test]
fn le_menu_filtre_par_usage_et_par_base() {
    let mut app = app_avec_menu();
    let _ = app.update(Message::ToggleMenu);
    let _ = app.update(Message::MenuBase(Some("debian".to_owned())));
    assert!(!contient(&app, "Ubuntu 26.04.1 desktop LTS"));
    let _ = app.update(Message::MenuBase(Some("ubuntu".to_owned())));
    let _ = app.update(Message::MenuCategory(Some("office".to_owned())));
    assert!(contient(&app, "Ubuntu 26.04.1 desktop LTS"));
    assert_eq!(
        app.menu_filter.base.as_deref(),
        Some("ubuntu"),
        "changer d'usage garde la base"
    );
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
    assert!(contient(
        &app,
        "Catalogue indisponible. Détail technique (en anglais) : expired"
    ));
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
        model: Model {
            auto_select: true,
            ..Model::default()
        },
        ..App::default()
    };
    let _ = app.update(Message::Loaded(Ok(sample())));
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // Writing stays a deliberate act: nothing starts, and a deselection holds despite the refresh.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Ok(sample())));
    assert_eq!(app.selected, None);
    let mut deux = App {
        model: Model {
            auto_select: true,
            allow_virtual: false,
            show_internal: true,
            ..Model::default()
        },
        ..App::default()
    };
    let _ = deux.update(Message::Loaded(Ok(sample())));
    assert_eq!(deux.selected, None);
}

#[test]
fn tor_propose_seulement_s_il_est_la_jamais_coche_seul() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ToggleTor(true));
    assert!(!app.use_tor, "sans Tor détecté, la case reste vide");
    let _ = app.update(Message::TorDetected(Some(
        "socks5h://127.0.0.1:9050".to_owned(),
    )));
    assert!(!app.use_tor, "détecté n'est pas activé");
    let _ = app.update(Message::ToggleTor(true));
    assert!(app.use_tor);
}

#[test]
fn la_langue_se_choisit_dans_les_parametres_et_relance() {
    use bonk_ui::settings::{LangChoice, SettingsMessage};
    let mut app = app_avec_panneau();
    assert!(app.status_line(1).contains("disque(s)"));
    let _ = app.update(Message::Settings(SettingsMessage::Open));
    let _ = app.update(Message::Settings(SettingsMessage::Language(LangChoice::En)));
    assert_eq!(app.lang, bonk_hal::Lang::Fr);
    assert_eq!(app.lang_choice, LangChoice::En);
    assert_eq!(app.focus, Some(Focus::Language(LangChoice::En)));
}
