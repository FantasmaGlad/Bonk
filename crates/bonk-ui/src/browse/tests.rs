use super::*;
use crate::catalog::Choice;
use crate::model::Message;

fn debian() -> System {
    let download = |id: &str| Choice {
        id: id.to_owned(),
        name: "Debian".to_owned(),
        label: "13".to_owned(),
        end_of_life: false,
        note: None,
        source: ChoiceSource::Download {
            url: format!("https://d/{id}.iso"),
            sha256: "ab".repeat(32),
        },
        desktop: Some("gnome".to_owned()),
        media: Some("usb".to_owned()),
    };
    let dvd = Choice {
        id: "debian-dvd".to_owned(),
        source: ChoiceSource::OfficialPage("https://www.debian.org/".to_owned()),
        ..download("debian-dvd")
    };
    System {
        name: "Debian".to_owned(),
        uses: vec!["office".to_owned()],
        base: Some("debian".to_owned()),
        icon: None,
        choices: vec![download("debian-live"), dvd],
    }
}

fn browse(message: BrowseMessage, model: &mut Model) -> Effects {
    model.update(Message::Browse(message))
}

#[test]
fn l_onglet_lit_le_catalogue_une_fois_et_jamais_hors_ligne() {
    let mut model = Model::default();
    assert_eq!(
        browse(BrowseMessage::Open, &mut model),
        [Effect::LoadCatalog]
    );
    assert_eq!(model.screen, Screen::Catalog);
    assert!(browse(BrowseMessage::Open, &mut model).is_empty());
    let mut offline = Model {
        offline: true,
        ..Model::default()
    };
    assert!(browse(BrowseMessage::Open, &mut offline).is_empty());
    assert_eq!(offline.screen, Screen::Main);
}

#[test]
fn ecrire_une_variante_revient_aux_disques_avec_l_image_demandee() {
    let mut model = Model {
        catalog: CatalogState::Ready(vec![debian()]),
        ..Model::default()
    };
    let _ = browse(BrowseMessage::Open, &mut model);
    let _ = browse(BrowseMessage::OpenSystem("Debian".to_owned()), &mut model);
    assert_eq!(
        model.catalog_focus_order(),
        [
            Focus::CloseCatalog,
            Focus::CloseSystem,
            Focus::Variant("debian-live".to_owned())
        ]
    );
    let _ = browse(BrowseMessage::Write("debian-dvd".to_owned()), &mut model);
    assert_eq!(
        model.screen,
        Screen::Catalog,
        "une page officielle ne s'écrit pas"
    );
    let _ = browse(BrowseMessage::Write("debian-live".to_owned()), &mut model);
    assert_eq!(model.screen, Screen::Main);
    assert_eq!(model.requested.as_deref(), Some("debian-live"));
    assert_eq!(model.browse.opened, None);
}

#[test]
fn fermer_un_systeme_rend_le_focus_a_sa_carte() {
    let mut model = Model {
        catalog: CatalogState::Ready(vec![debian()]),
        ..Model::default()
    };
    let _ = browse(BrowseMessage::OpenSystem("Debian".to_owned()), &mut model);
    let _ = browse(BrowseMessage::CloseSystem, &mut model);
    assert_eq!(model.focus, Some(Focus::System("Debian".to_owned())));
    assert_eq!(model.opened_system(), None);
}
