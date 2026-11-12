use super::*;
use bonk_ui::library::{LibraryMessage, parse};
use bonk_ui::model::WriteState;

const SHA: &str = "0f7d0e1fa2b5d64e1c0a3e8a4f42ac1b5e0d37a1c2e7f62f6c8c4b0f0a1e9d33";

pub(super) fn avec_une_image() -> App {
    let mut app = loaded(sample());
    let liste = format!(
        r#"{{"directory":"/cache","entries":[
            {{"name":"debian-13-amd64-netinst.iso","sha256":"{SHA}","bytes":786432000,"partial":false,"modified":1794441600}},
            {{"name":"ubuntu.iso.part","sha256":"{}","bytes":1048576,"partial":true,"modified":1794400000}}
        ]}}"#,
        "1".repeat(64)
    );
    let _ = app.update(Message::Library(LibraryMessage::Open));
    let _ = app.update(Message::Library(LibraryMessage::Listed(parse(
        liste.as_bytes(),
    ))));
    app
}

#[test]
fn la_bibliotheque_montre_chaque_image_gardee() {
    let app = avec_une_image();
    let faits = format!(
        "{} · 2026-11-12",
        bonk_hal::format_size(786_432_000, bonk_hal::Lang::Fr)
    );
    let coupee = format!(
        "{} · 2026-11-11 · téléchargement incomplet",
        bonk_hal::format_size(1_048_576, bonk_hal::Lang::Fr)
    );
    for present in [
        "Bibliothèque",
        "debian-13-amd64-netinst.iso",
        &faits,
        &coupee,
        "Supprimer",
    ] {
        assert!(contient(&app, present), "{present}");
    }
    assert!(!contient(&app, "Cle de test"));
}

#[test]
fn ecrire_depuis_la_bibliotheque_remplit_le_panneau_avec_l_empreinte() {
    let mut app = avec_une_image();
    let _ = app.update(Message::Library(LibraryMessage::Write(SHA.to_owned())));
    assert!(contient(
        &app,
        "debian-13-amd64-netinst.iso est choisie : sélectionnez le disque où l'écrire."
    ));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.library.chosen, None);
    let WriteState::Draft { image, hash, .. } = &app.write else {
        panic!("panneau d'écriture attendu");
    };
    assert!(image.ends_with("debian-13-amd64-netinst.iso"), "{image}");
    assert_eq!(hash, SHA);
    assert!(
        app.equivalents()
            .iter()
            .any(|command| command.contains(&format!("--sha256 {SHA}")))
    );
}
