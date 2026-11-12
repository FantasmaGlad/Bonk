use super::menu::versions_du_catalogue;
use super::panneau::app_avec_panneau;
use super::rendu::contient;
use super::*;

const UBUNTU: &str = "ubuntu-26.04-desktop-x86_64";

fn brouillon(app: &App) -> Option<(&str, &str)> {
    match &app.write {
        WriteState::Draft { image, hash, .. } => Some((image.as_str(), hash.as_str())),
        _ => None,
    }
}

fn catalogue_lu(app: &mut App) {
    let _ = app.update(Message::CatalogLoaded(Ok(versions_du_catalogue())));
}

#[test]
fn une_image_demandee_lit_le_catalogue_sans_attendre_un_disque() {
    let mut app = App::default();
    let effects = app.model.request_image(UBUNTU.to_owned());
    assert!(effects.contains(&Effect::LoadCatalog), "{effects:?}");
    assert!(contient(
        &app,
        "Lecture du catalogue pour trouver ubuntu-26.04-desktop-x86_64..."
    ));
    // Asked again while it loads: one read only.
    assert!(app.model.request_image(UBUNTU.to_owned()).is_empty());
}

#[test]
fn sans_disque_l_image_attend_et_le_dit_puis_remplit_le_brouillon() {
    let mut app = App::default();
    let _ = app.model.request_image(UBUNTU.to_owned());
    catalogue_lu(&mut app);
    assert!(matches!(app.write, WriteState::Idle));
    assert!(contient(
        &app,
        "Ubuntu 26.04.1 desktop LTS est choisi : sélectionnez le disque où l'écrire."
    ));
    let _ = app.update(Message::Loaded(Ok(sample())));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(
        brouillon(&app),
        Some(("https://u/26.iso", "a".repeat(64).as_str()))
    );
    assert!(!contient(&app, "sélectionnez le disque où l'écrire"));
    assert!(contient(&app, "Ubuntu 26.04.1 desktop LTS"));
}

#[test]
fn un_disque_deja_choisi_recoit_l_image_a_la_lecture_du_catalogue() {
    let mut app = app_avec_panneau();
    let _ = app.model.request_image(UBUNTU.to_owned());
    assert_eq!(brouillon(&app), Some(("", "")));
    catalogue_lu(&mut app);
    assert_eq!(
        brouillon(&app),
        Some(("https://u/26.iso", "a".repeat(64).as_str()))
    );
}

#[test]
fn une_image_demandee_n_ecrit_jamais_d_elle_meme() {
    let mut app = app_avec_panneau();
    let effects = app.model.request_image(UBUNTU.to_owned());
    assert!(!effects.iter().any(|e| matches!(e, Effect::StartWrite(_))));
    let effects = app
        .model
        .update(Message::CatalogLoaded(Ok(versions_du_catalogue())));
    assert!(effects.is_empty(), "{effects:?}");
    assert!(matches!(app.write, WriteState::Draft { .. }));
    // The delay to think still holds: a click right away starts nothing.
    let effects = app.model.update(Message::StartWrite);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(matches!(app.write, WriteState::Draft { .. }));
}

#[test]
fn le_choix_de_l_utilisateur_l_emporte_sur_l_image_demandee() {
    let mut app = app_avec_panneau();
    let _ = app.update(Message::ImageInput("/tmp/autre.iso".to_owned()));
    let _ = app.model.request_image(UBUNTU.to_owned());
    catalogue_lu(&mut app);
    assert_eq!(
        brouillon(&app).map(|(image, _)| image),
        Some("/tmp/autre.iso")
    );

    let mut app = app_avec_panneau();
    let _ = app.model.request_image(UBUNTU.to_owned());
    catalogue_lu(&mut app);
    let autre = versions_du_catalogue()[0].choices[1].clone();
    let _ = app.update(Message::MenuChose(autre));
    assert_eq!(app.requested, None);
    // Another disk opens an empty draft: the request no longer applies.
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(brouillon(&app), Some(("", "")));
}

#[test]
fn l_image_demandee_suit_un_changement_de_disque() {
    let mut app = app_avec_panneau();
    let _ = app.model.request_image(UBUNTU.to_owned());
    catalogue_lu(&mut app);
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert!(matches!(app.write, WriteState::Idle));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(
        brouillon(&app).map(|(image, _)| image),
        Some("https://u/26.iso")
    );
}

#[test]
fn une_image_inconnue_ou_non_inscriptible_est_dite_et_ne_remplit_rien() {
    let mut app = app_avec_panneau();
    let _ = app.model.request_image("absente-1".to_owned());
    catalogue_lu(&mut app);
    assert_eq!(brouillon(&app), Some(("", "")));
    assert!(contient(
        &app,
        "Le catalogue n'a pas d'image nommée absente-1."
    ));

    let mut app = app_avec_panneau();
    let _ = app.model.request_image("ubuntu-11".to_owned());
    catalogue_lu(&mut app);
    assert_eq!(brouillon(&app), Some(("", "")));
    assert!(contient(
        &app,
        "Ubuntu 11 ne peut pas être récupéré et écrit sur une clé par Bonk."
    ));
}

#[test]
fn un_catalogue_illisible_est_dit_pour_l_image_demandee() {
    let mut app = App::default();
    let _ = app.model.request_image(UBUNTU.to_owned());
    let _ = app.update(Message::CatalogLoaded(Err("hors ligne".to_owned())));
    assert!(contient(
        &app,
        "Le catalogue n'a pas pu être lu : ubuntu-26.04-desktop-x86_64 n'est pas choisi."
    ));
}

#[test]
fn un_lien_recu_pendant_que_la_fenetre_tourne_choisit_l_image() {
    let mut app = app_avec_panneau();
    catalogue_lu(&mut app);
    let _ = app.update(Message::RequestImage(UBUNTU.to_owned()));
    assert_eq!(app.requested.as_deref(), Some(UBUNTU));
    assert_eq!(
        brouillon(&app),
        Some(("https://u/26.iso", "a".repeat(64).as_str()))
    );
}
