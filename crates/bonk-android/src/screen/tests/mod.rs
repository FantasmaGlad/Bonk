use std::time::{Duration, Instant};

use bonk_core::write::Phase;
use bonk_hal::{Disk, Lang};
use bonk_ui::catalog::{CatalogState, ChoiceSource};
use bonk_ui::event::{Event, Outcome};
use bonk_ui::model::{Effect, Message, Model, WriteState};
use bonk_ui::vault::{VaultAction, VaultMessage, VaultState};
use serde_json::{Value, json};

use super::drives::*;
use super::effects::*;
use super::messages::*;
use super::state::vault_state;
use super::*;

mod coffre;
mod disques;
mod ecriture;
mod protocole;
mod telechargement;

fn usb() -> Value {
    json!({"device": "/dev/bus/usb/001/002", "model": "UDisk", "serial": "S1", "size_bytes": 32_000_000_000_u64})
}

fn loaded() -> Model {
    let mut model = Model {
        auto_select: true,
        ..Model::default()
    };
    let disks: Vec<Disk> = disk_of(&usb()).into_iter().collect();
    let _ = model.update(Message::Loaded(Ok(disks)));
    model
}

#[test]
fn un_lien_choisit_une_image_du_catalogue_sans_rien_ecrire() {
    let mut model = loaded();
    let link = json!({"type": "open", "link": "bonk://install/debian-13-netinst-x86_64"});
    // The open draft already asked for the catalog: the link starts no second read.
    assert!(open_link(&mut model, &link).is_empty());
    assert_eq!(
        open_link(&mut Model::default(), &link),
        vec![Effect::LoadCatalog { proxy: None }]
    );
    assert_eq!(model.requested.as_deref(), Some("debian-13-netinst-x86_64"));
    assert!(matches!(model.write, WriteState::Draft { .. }));
    for refused in [
        json!({"type": "open", "link": "bonk://install/x?disk=usb-S1"}),
        json!({"type": "open", "link": "https://example.org/x.iso"}),
        json!({"type": "open"}),
        json!({"type": "tick", "link": "bonk://install/other"}),
    ] {
        assert!(open_link(&mut model, &refused).is_empty(), "{refused}");
        assert_eq!(model.requested.as_deref(), Some("debian-13-netinst-x86_64"));
    }
}

/// A panic while holding the state must not leave the screen dead: the poisoned lock is taken back with
/// the state as it was, instead of answering unavailable to every later message.
#[test]
fn un_verrou_empoisonne_est_repris() {
    let etat = std::sync::Arc::new(Mutex::new(1));
    let partage = std::sync::Arc::clone(&etat);
    let _ = std::thread::spawn(move || {
        let _garde = partage.lock();
        panic!("panique pendant la mise à jour");
    })
    .join();
    assert!(etat.is_poisoned());
    assert_eq!(*lock(&etat), 1);
}

#[test]
fn l_etat_dit_la_langue_choisie() {
    let mut model = loaded();
    model.lang = Lang::Fr;
    assert_eq!(state(&model)["lang"], "fr");
    // A new language comes by a relaunch, never while the screen runs.
    let input = json!({"type": "settings", "action": "language", "value": "en"});
    let effects = model.update(message_of(&input, &model).unwrap_or(Message::Tick));
    assert_eq!(
        effects_out(&mut model, effects, None),
        [json!({"do": "relaunch", "key": "lang", "value": "en"})]
    );
    assert_eq!(state(&model)["lang"], "fr");
}
