use super::*;

#[test]
fn le_bouton_attend_l_image_et_les_trois_secondes() {
    let mut model = loaded();
    assert_eq!(state(&model)["panel"]["enabled"], false);
    let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
    assert_eq!(
        state(&model)["panel"]["enabled"],
        false,
        "délai de réflexion"
    );
    if let WriteState::Draft { opened, .. } = &mut model.write {
        *opened = Instant::now()
            .checked_sub(Duration::from_secs(4))
            .unwrap_or(*opened);
    }
    assert_eq!(state(&model)["panel"]["enabled"], true);
}

#[test]
fn un_depart_demande_a_kotlin_d_ecrire_avec_le_brouillon() {
    let mut model = loaded();
    let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
    if let WriteState::Draft { opened, .. } = &mut model.write {
        *opened = Instant::now()
            .checked_sub(Duration::from_secs(4))
            .unwrap_or(*opened);
    }
    let draft = draft_of(&model);
    let effects = model.update(Message::StartWrite);
    let out = effects_out(&mut model, effects, draft);
    assert_eq!(out[0]["do"], "write");
    assert_eq!(out[0]["device"], "/dev/bus/usb/001/002");
    assert!(matches!(model.write, WriteState::Running { .. }));
}

/// A failure born in the app (cancel, key gone) carries a Bonk code: the screen shows its translated
/// title and code, as for an engine error.
#[test]
fn un_echec_de_l_application_porte_son_code() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = loaded();
    let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
    if let WriteState::Draft { opened, .. } = &mut model.write {
        *opened = Instant::now()
            .checked_sub(Duration::from_secs(4))
            .unwrap_or(*opened);
    }
    let _ = model.update(Message::StartWrite);
    let _ = model.update(Message::Write(Event::Started(0)));
    let failed = json!({"type": "failed", "code": "E4003", "message": "cancelled"});
    let message = message_of(&failed, &model).ok_or("message refusé")?;
    let _ = model.update(message);
    let shown = state(&model)["panel"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(shown.contains("[E4003]"), "{shown}");
    Ok(())
}
