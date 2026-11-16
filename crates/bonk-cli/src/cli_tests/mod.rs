use std::cell::RefCell;

use bonk_hal::{Bus, DiskKind, Partition};
use serde_json::Value;

use super::*;
use crate::command_line::split_words;
use crate::errors::EXIT_RUNTIME_ERROR;

mod config;
mod contract;
mod diagnostics;
mod listing;
mod window;

fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
    Partition {
        path: path.to_owned(),
        label: None,
        fs_type: Some("ext4".to_owned()),
        size_bytes: 1 << 30,
        mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
        holders: vec![],
        is_system_volume: system,
        is_active_esp: false,
    }
}

fn disk(id: &str, path: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
    Disk {
        id: id.to_owned(),
        path: path.to_owned(),
        model: "Modèle de test".to_owned(),
        serial: None,
        size_bytes: 16_008_609_792,
        bus,
        kind: DiskKind::Physical,
        removable_flag: false,
        logical_block_size: None,
        location: None,
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
            Bus::Nvme,
            vec![partition("/dev/nvme0n1p2", &["/"], true)],
        ),
        disk("usb-cle", "/dev/sdb", Bus::Usb, vec![]),
        disk("sata-libre", "/dev/sda", Bus::Sata, vec![]),
    ]
}

/// What `execute` did: code, outputs, log settings received.
struct Resultat {
    code: u8,
    out: String,
    err: String,
    journal_installe: bool,
    fenetre_ouverte: bool,
}

fn lancer_avec(
    args: &[&str],
    env_log: Option<&str>,
    disks: Vec<Disk>,
    fenetre: Result<(), String>,
) -> Resultat {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let journal = RefCell::new(false);
    let ouverte = RefCell::new(false);
    let code = execute(
        std::iter::once("bonk").chain(args.iter().copied()),
        &Env {
            bonk_log: env_log.map(str::to_owned),
            ..Env::default()
        },
        &mut out,
        &mut err,
        |_| {
            *journal.borrow_mut() = true;
            Ok(())
        },
        || Ok(disks.clone()),
        |_| {
            *ouverte.borrow_mut() = true;
            fenetre
        },
    );
    Resultat {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
        journal_installe: *journal.borrow(),
        fenetre_ouverte: *ouverte.borrow(),
    }
}

fn lancer(args: &[&str], disks: Vec<Disk>) -> (u8, String, String) {
    let r = lancer_avec(args, None, disks, Ok(()));
    (r.code, r.out, r.err)
}
