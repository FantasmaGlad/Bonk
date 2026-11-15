use super::*;

#[test]
fn la_cle_passe_le_meme_filtre_que_sur_pc_et_se_selectionne() {
    let model = loaded();
    assert_eq!(model.selected.as_deref(), Some("usb-S1-001-002"));
    let state = state(&model);
    assert_eq!(state["disks"][0]["model"], "UDisk");
    assert_eq!(state["panel"]["kind"], "draft");
}

/// A row that changes height at each detection makes the whole screen jump.
#[test]
fn la_detection_periodique_ne_change_pas_l_etat_affiche() {
    let mut model = Model {
        auto_select: true,
        ..Model::default()
    };
    let _ = model.update(Message::Loaded(Ok(Vec::new())));
    let before = state(&model);
    let _ = model.update(Message::Tick);
    assert!(model.loading);
    assert_eq!(state(&model), before);
    let _ = model.update(Message::Loaded(Ok(Vec::new())));
    assert_eq!(state(&model), before);
}

/// Cloned keys often share a serial: each must keep its own card, and tapping the second must write the
/// second, never the first in the list.
#[test]
fn deux_cles_de_meme_serie_restent_distinctes() {
    let mut model = Model {
        auto_select: false,
        ..Model::default()
    };
    let mut b = usb();
    b["device"] = json!("/dev/bus/usb/001/003");
    let disks: Vec<Disk> = [usb(), b].iter().filter_map(disk_of).collect();
    let _ = model.update(Message::Loaded(Ok(disks)));
    let shown = disks_state(&model);
    assert_eq!(shown.len(), 2);
    assert_ne!(shown[0]["id"], shown[1]["id"]);
    let second = shown
        .iter()
        .find(|disk| disk["device"] == "/dev/bus/usb/001/003")
        .and_then(|disk| disk["id"].as_str())
        .map(str::to_owned)
        .unwrap_or_default();
    let _ = model.update(Message::Select(second));
    let selected: Vec<Value> = disks_state(&model)
        .into_iter()
        .filter(|disk| disk["selected"] == true)
        .map(|disk| disk["device"].clone())
        .collect();
    assert_eq!(selected, vec![json!("/dev/bus/usb/001/003")]);
    let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
    assert_eq!(
        draft_of(&model).map(|draft| draft["device"].clone()),
        Some(json!("/dev/bus/usb/001/003"))
    );
}

/// The summary before erasing says what the key holds: a table read, never 0 by default.
#[test]
fn les_partitions_lues_sur_la_cle_arrivent_a_l_ecran() -> Result<(), Box<dyn std::error::Error>> {
    use bonk_core::MemoryDevice;
    use bonk_core::format::table::{TableKind, write_table};
    use bonk_core::format::{Filesystem, Ids};

    let mut dev = MemoryDevice::new(64 << 20, 512)?;
    write_table(
        &mut dev,
        TableKind::Gpt,
        Filesystem::Fat32,
        Some("DONNEES"),
        &Ids::from_seed(b"cle"),
        false,
    )?;
    let partitions = partitions_of(&bonk_core::partition::inspect(&dev)?);
    assert_eq!(partitions.len(), 1);
    let mut value = usb();
    value["partitions"] = json!(partitions);
    let mut model = Model {
        auto_select: true,
        ..Model::default()
    };
    let _ = model.update(Message::Loaded(Ok(disk_of(&value).into_iter().collect())));
    // The partition read on the key reaches the screen: its label is on the card.
    let facts = state(&model)["disks"][0]["facts"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(facts.contains("DONNEES"), "{facts}");
    // A blank key really has no partition.
    let blank = MemoryDevice::new(64 << 20, 512)?;
    assert!(partitions_of(&bonk_core::partition::inspect(&blank)?).is_empty());
    Ok(())
}

#[test]
fn une_cle_de_taille_inconnue_n_est_pas_un_disque() {
    assert!(disk_of(&json!({"device": "d", "model": "m"})).is_none());
}
