#![allow(clippy::expect_used)]

use std::time::Instant;

use bonk_hal::Lang;
use serde_json::json;

use crate::command::ViewOptions;
use crate::model::{Effect, Message, Model, WriteState};

const LOCAL: &str = "/home/alice/images/dvd.iso";

fn drafting(image: &str, raw: bool, lang: Lang) -> Model {
    let mut model = Model::new(ViewOptions::default(), lang).0;
    model.write = WriteState::Draft {
        disk_id: "usb-a".to_owned(),
        image: image.to_owned(),
        hash: String::new(),
        raw,
        mbr: false,
        opened: Instant::now(),
    };
    model
}

fn report(refusal: Option<&str>) -> serde_json::Value {
    json!({
        "route": if refusal.is_some() { json!(null) } else { json!("raw") },
        "refusal": refusal,
        "efi_loaders": [{"machine": "x86_64", "signer": "unsigned"}],
    })
}

#[test]
fn une_image_locale_est_inspectee_une_fois_puis_dit_secure_boot() {
    let mut model = drafting(LOCAL, false, Lang::En);
    assert_eq!(
        model.update(Message::Clock),
        [Effect::InspectImage(LOCAL.to_owned())]
    );
    assert_eq!(model.update(Message::Clock), []);
    assert_eq!(model.boot_line(), None);
    let _ = model.update(Message::Inspected(LOCAL.to_owned(), Some(report(None))));
    let line = model.boot_line().expect("Secure Boot");
    assert!(line.starts_with("Secure Boot: unsigned loader"), "{line}");
}

#[test]
fn une_adresse_ou_un_champ_vide_n_est_pas_inspecte() {
    for image in ["https://example.org/debian.iso", "  "] {
        let mut model = drafting(image, false, Lang::Fr);
        assert_eq!(model.update(Message::Clock), [], "{image}");
    }
}

#[test]
fn un_refus_se_dit_avant_l_ecriture_sauf_en_mode_brut() {
    let refusal = "ISO without disk boot: it would not boot from a USB drive";
    for (raw, expected) in [
        (false, format!("Bonk refuse cette image : {refusal}")),
        (true, "Secure Boot : chargeur non signé".to_owned()),
    ] {
        let mut model = drafting(LOCAL, raw, Lang::Fr);
        let _ = model.update(Message::Clock);
        let _ = model.update(Message::Inspected(
            LOCAL.to_owned(),
            Some(report(Some(refusal))),
        ));
        let line = model.boot_line().expect("ligne");
        assert!(line.starts_with(&expected), "{line}");
    }
}

#[test]
fn la_reponse_d_une_autre_image_ou_un_echec_ne_dit_rien() {
    let mut model = drafting(LOCAL, false, Lang::En);
    let _ = model.update(Message::Clock);
    let _ = model.update(Message::Inspected(
        "/autre.iso".to_owned(),
        Some(report(None)),
    ));
    assert_eq!(model.boot_line(), None);
    let _ = model.update(Message::Inspected(LOCAL.to_owned(), None));
    assert_eq!(model.boot_line(), None);
    if let WriteState::Draft { image, .. } = &mut model.write {
        "/home/alice/images/autre.iso".clone_into(image);
    }
    assert_eq!(model.boot_line(), None);
    assert_eq!(
        model.update(Message::Clock),
        [Effect::InspectImage(
            "/home/alice/images/autre.iso".to_owned()
        )]
    );
}
