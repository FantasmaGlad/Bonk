use super::*;

fn contient_a(app: &App, largeur: f32, texte: &str) -> bool {
    Simulator::with_size(
        iced::Settings::default(),
        Size::new(largeur, 640.0),
        app.view(),
    )
    .find(texte)
    .is_ok()
}

#[test]
fn large_le_panneau_est_a_cote_et_attend_un_disque() {
    let app = loaded(sample());
    assert!(contient_a(
        &app,
        960.0,
        "Choisissez un disque pour y écrire une image."
    ));
    assert!(!contient_a(
        &app,
        700.0,
        "Choisissez un disque pour y écrire une image."
    ));
}

#[test]
fn etroite_ou_large_le_panneau_du_disque_choisi_s_affiche() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    for largeur in [700.0, 960.0] {
        assert!(contient_a(&app, largeur, "Cle de test"), "{largeur}");
        assert!(
            contient_a(&app, largeur, "ÉCRIRE UNE IMAGE SUR CE DISQUE"),
            "{largeur}"
        );
    }
}
