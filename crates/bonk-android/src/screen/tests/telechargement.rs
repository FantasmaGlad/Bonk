#![allow(clippy::expect_used)]

use bonk_ui::browse::{BrowseMessage, DownloadState};
use bonk_ui::catalog::{Choice, System};
use bonk_ui::settings::Screen;

use super::*;

const SHA256: &str = "ab";

fn on_debian_page() -> Model {
    let choice = Choice {
        id: "debian-live".to_owned(),
        name: "Debian".to_owned(),
        label: "13".to_owned(),
        end_of_life: false,
        note: None,
        source: ChoiceSource::Download {
            url: "https://d/debian-live.iso".to_owned(),
            sha256: SHA256.to_owned(),
        },
        desktop: None,
        media: None,
    };
    let mut model = loaded();
    model.lang = Lang::Fr;
    model.catalog = CatalogState::Ready(vec![System {
        name: "Debian".to_owned(),
        uses: Vec::new(),
        base: None,
        icon: None,
        choices: vec![choice],
    }]);
    model.screen = Screen::Catalog;
    model.browse.opened = Some("Debian".to_owned());
    model
}

fn start(model: &mut Model) -> Vec<Value> {
    let input = json!({"type": "catalog_download", "action": "start", "id": "debian-live"});
    let message = message_of(&input, model).expect("compris");
    let effects = model.update(message);
    effects_out(model, effects, None)
}

#[test]
fn l_adresse_et_l_empreinte_viennent_du_catalogue_signe() {
    let mut model = on_debian_page();
    assert_eq!(
        start(&mut model),
        [json!({
            "do": "download_image",
            "url": "https://d/debian-live.iso",
            "sha256": SHA256,
            "name": "debian-live.iso",
            "proxy": null,
        })]
    );
    assert!(start(&mut model).is_empty(), "un seul à la fois");
}

#[test]
fn la_progression_et_l_echec_vont_au_telechargement_en_cours() {
    let mut model = on_debian_page();
    let _ = start(&mut model);
    let progress = json!({"type": "download", "done": 5, "total": 10, "rate": 1.0});
    assert!(matches!(
        message_of(&progress, &model),
        Some(Message::Browse(BrowseMessage::DownloadEvent(
            Event::Progress { done: 5, .. }
        )))
    ));
    let start_write = json!({"type": "start"});
    assert!(message_of(&start_write, &model).is_none(), "la clé attend");
    let failed = json!({"type": "failed", "message": "coupure", "code": "E3009"});
    let message = message_of(&failed, &model).expect("compris");
    let _ = model.update(message);
    assert!(matches!(
        model.browse.download,
        Some(DownloadState::Finished {
            outcome: Outcome::Failed { .. },
            ..
        })
    ));
}

#[test]
fn l_image_enregistree_dit_ou_elle_se_trouve() {
    let mut model = on_debian_page();
    let _ = start(&mut model);
    let saved = json!({"type": "downloaded", "path": "Download/debian-live.iso", "bytes": 3_000_000_000_u64});
    let message = message_of(&saved, &model).expect("compris");
    let _ = model.update(message);
    let page = crate::browse::state(&model);
    assert_eq!(
        page["system"]["downloading"]["line"],
        "Image téléchargée et contrôlée, enregistrée dans Download/debian-live.iso (3,0 Go)."
    );
    assert_eq!(page["system"]["downloading"]["action"], "dismiss");
    assert_eq!(page["system"]["busy"], false);
}

#[test]
fn un_telechargement_attend_la_fin_d_une_ecriture() {
    let mut model = on_debian_page();
    model.write = WriteState::Running {
        disk_id: "usb-S1".to_owned(),
        pid: Some(0),
        phase: Phase::Write,
        done: 0,
        total: None,
        rate: 0.0,
    };
    let input = json!({"type": "catalog_download", "action": "start", "id": "debian-live"});
    assert!(message_of(&input, &model).is_none());
}
