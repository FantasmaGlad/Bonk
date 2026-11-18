use super::*;
use crate::catalog::Choice;
use crate::event::{Event, Outcome};
use crate::model::{Effect, Message};

fn debian() -> System {
    let download = |id: &str| Choice {
        id: id.to_owned(),
        name: "Debian".to_owned(),
        edition: None,
        label: "13".to_owned(),
        end_of_life: false,
        note: None,
        source: ChoiceSource::Download {
            url: format!("https://d/{id}.iso"),
            sha256: "ab".repeat(32),
        },
        desktop: Some("gnome".to_owned()),
        media: "usb".to_owned(),
        arch: "x86_64".to_owned(),
    };
    let dvd = Choice {
        id: "debian-dvd".to_owned(),
        source: ChoiceSource::OfficialPage("https://www.debian.org/".to_owned()),
        ..download("debian-dvd")
    };
    System {
        id: String::new(),
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
        [Effect::LoadCatalog { proxy: None }]
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
            Focus::Variant("debian-live".to_owned()),
            Focus::Download("debian-live".to_owned())
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

fn on_debian_page() -> Model {
    let mut model = Model {
        catalog: CatalogState::Ready(vec![debian()]),
        ..Model::default()
    };
    let _ = browse(BrowseMessage::Open, &mut model);
    let _ = browse(BrowseMessage::OpenSystem("Debian".to_owned()), &mut model);
    model
}

#[test]
fn telecharger_une_variante_reste_sur_sa_page_et_une_seule_a_la_fois() {
    let mut model = on_debian_page();
    assert!(browse(BrowseMessage::Download("debian-dvd".to_owned()), &mut model).is_empty());
    assert_eq!(
        browse(
            BrowseMessage::Download("debian-live".to_owned()),
            &mut model
        ),
        [Effect::RunDownload(download_args("debian-live", None))]
    );
    assert_eq!(model.screen, Screen::Catalog);
    assert!(
        browse(
            BrowseMessage::Download("debian-live".to_owned()),
            &mut model
        )
        .is_empty()
    );
    let _ = browse(BrowseMessage::DownloadEvent(Event::Started(7)), &mut model);
    assert_eq!(
        browse(BrowseMessage::CancelDownload, &mut model),
        [Effect::CancelWrite(7)]
    );
    assert!(
        model
            .browse
            .download
            .as_ref()
            .is_some_and(DownloadState::running)
    );
    let _ = browse(BrowseMessage::DismissDownload, &mut model);
    assert!(
        model
            .browse
            .download
            .as_ref()
            .is_some_and(DownloadState::running),
        "un téléchargement en cours ne se ferme pas"
    );
}

#[test]
fn une_image_gardee_rafraichit_la_bibliotheque() {
    let mut model = on_debian_page();
    let _ = browse(
        BrowseMessage::Download("debian-live".to_owned()),
        &mut model,
    );
    let done = Outcome::Downloaded {
        path: "/c/debian-live.iso".to_owned(),
        bytes: 3,
    };
    assert_eq!(
        browse(
            BrowseMessage::DownloadEvent(Event::Finished(done.clone())),
            &mut model
        ),
        [Effect::ListKept]
    );
    assert_eq!(
        model.browse.download,
        Some(DownloadState::Finished {
            id: "debian-live".to_owned(),
            outcome: done
        })
    );
    let _ = browse(BrowseMessage::DismissDownload, &mut model);
    assert_eq!(model.browse.download, None);
}

#[test]
fn le_telechargement_passe_par_tor_quand_il_est_choisi() {
    assert!(
        download_args("x", Some("socks5h://127.0.0.1:9050"))
            .windows(2)
            .any(|pair| pair == ["--proxy", "socks5h://127.0.0.1:9050"])
    );
}

#[test]
fn le_resultat_de_la_commande_se_lit_comme_une_image_gardee() {
    let line = r#"{"schema_version":1,"event":"result","download":{"id":"x","path":"/c/x.iso","sha256":"ab","bytes":3,"already_kept":false}}"#;
    assert_eq!(
        crate::event::parse_line(line),
        Some(Event::Finished(Outcome::Downloaded {
            path: "/c/x.iso".to_owned(),
            bytes: 3
        }))
    );
}
