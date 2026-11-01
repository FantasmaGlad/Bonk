use super::view::secondes_restantes;
use bonk_hal::{Bus, DiskKind, Partition};
use iced::keyboard::{Key, key::Named};
use iced_test::Simulator;

use super::navigation::*;
use std::time::Instant;

use bonk_hal::Disk;
use bonk_ui::model::ui_event;

use super::*;

mod clavier;
mod commandes;
mod menu;
mod panneau;
mod rendu;
mod resultat;

use clavier::*;
use panneau::*;
use rendu::*;

fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
    Partition {
        path: path.to_owned(),
        label: Some("CLE".to_owned()),
        fs_type: Some("vfat".to_owned()),
        size_bytes: 15_000_000_000,
        mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
        holders: vec![],
        is_system_volume: system,
        is_active_esp: false,
    }
}

fn disk(id: &str, path: &str, model: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
    Disk {
        id: id.to_owned(),
        path: path.to_owned(),
        model: model.to_owned(),
        serial: Some("4C530001".to_owned()),
        size_bytes: 16_008_609_792,
        bus,
        kind: DiskKind::Physical,
        removable_flag: true,
        fs_type: None,
        holders: vec![],
        partitions,
    }
}

fn sample() -> Vec<Disk> {
    vec![
        disk(
            "nvme-systeme",
            "/dev/nvme0n1",
            "Disque systeme",
            Bus::Nvme,
            vec![partition("/dev/nvme0n1p2", &["/"], true)],
        ),
        disk(
            "usb-cle",
            "/dev/sdb",
            "Cle de test",
            Bus::Usb,
            vec![partition("/dev/sdb1", &["/run/media/alice/CLE"], false)],
        ),
        disk(
            "sata-libre",
            "/dev/sda",
            "Disque interne libre",
            Bus::Sata,
            vec![],
        ),
    ]
}

fn loaded(disks: Vec<Disk>) -> App {
    let mut app = App::default();
    let _ = app.update(Message::Loaded(Ok(disks)));
    app
}

fn visible_ids(app: &App) -> Vec<&str> {
    app.entries().iter().map(|(d, _)| d.id.as_str()).collect()
}

#[test]
fn par_defaut_seuls_les_disques_proposes_sont_affiches() {
    assert_eq!(visible_ids(&loaded(sample())), ["usb-cle"]);
}

#[test]
fn les_cases_changent_l_affichage_sans_nouvelle_enumeration() {
    let mut app = loaded(sample());
    let _ = app.update(Message::ToggleAll(true));
    assert_eq!(visible_ids(&app), ["nvme-systeme", "usb-cle", "sata-libre"]);
    let _ = app.update(Message::ToggleAll(false));
    let _ = app.update(Message::ToggleInternal(true));
    assert_eq!(visible_ids(&app), ["usb-cle", "sata-libre"]);
    assert!(!app.loading);
}

#[test]
fn un_disque_systeme_n_est_jamais_affiche_comme_propose() {
    let mut app = loaded(sample());
    for message in [
        Message::ToggleAll(true),
        Message::ToggleInternal(true),
        Message::ToggleVirtual(true),
    ] {
        let _ = app.update(message);
    }
    let entries = app.entries();
    let system = entries.iter().find(|(d, _)| d.id == "nvme-systeme");
    assert!(system.is_some_and(|(_, verdict)| !verdict.is_writable()));
}

#[test]
fn la_selection_bascule_et_disparait_avec_le_disque() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.selected, None);
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    // La clé est débranchée : la sélection tombe.
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert_eq!(app.selected, None);
}

#[test]
fn une_enumeration_a_la_fois() {
    let mut app = App::default();
    let _ = app.update(Message::Refresh);
    assert!(app.loading);
    // Le minuteur ne relance rien tant que la précédente n'est pas finie : l'état reste « en cours ».
    let _ = app.update(Message::Tick);
    assert!(app.loading);
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert!(!app.loading);
}

#[test]
fn un_echec_vide_la_liste_au_lieu_de_la_garder() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Err("base udev introuvable".to_owned())));
    assert!(app.disks.is_empty());
    assert_eq!(app.selected, None);
    assert_eq!(app.error.as_deref(), Some("base udev introuvable"));
    // Une énumération réussie efface l'erreur.
    let _ = app.update(Message::Loaded(Ok(sample())));
    assert_eq!(app.error, None);
}
