//! Commande équivalente et journal.

use super::*;

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
