//! Clé d'installation Windows de bout en bout, par le binaire, sur un fichier image : ISO synthétique,
//! volume relu par `fsck.vfat`, outil de référence. Aucun disque n'est touché.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use bonk_core::udf::testing::build;

mod common;

struct Dossier(PathBuf);

impl Dossier {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bonk-win-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("dossier de test");
        Self(path)
    }
    fn file(&self, name: &str) -> String {
        self.0.join(name).display().to_string()
    }
}

impl Drop for Dossier {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bonk(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bonk"))
        .args(args)
        .output()
        .expect("lancement de bonk")
}

/// Chargeur x86_64 minimal, sans signature.
fn loader(machine: u16) -> Vec<u8> {
    let mut file = vec![0u8; 1024];
    file[..2].copy_from_slice(b"MZ");
    file[0x3C] = 0x40;
    file[0x40..0x44].copy_from_slice(b"PE\0\0");
    file[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    file[0x44 + 20..0x44 + 22].copy_from_slice(&0x20Bu16.to_le_bytes());
    file
}

fn windows_iso(machine: u16) -> Vec<u8> {
    let content: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    build(&[
        ("efi/boot/bootx64.efi", &loader(machine)),
        ("sources/install.wim", &content),
        ("Un nom assez long pour une entree longue.txt", b"contenu"),
        ("setup.exe", b"MZ"),
    ])
}

/// Le volume d'une image : tout ce qui suit le premier Mio, où commence la partition.
#[cfg(target_os = "linux")]
fn volume_of(dossier: &Dossier, image: &str) -> String {
    let bytes = fs::read(image).expect("lecture de l'image");
    let path = dossier.file("volume.fat");
    fs::write(&path, &bytes[1 << 20..]).expect("écriture du volume");
    path
}

// `fsck.vfat` n'existe en CI que sur Linux.
#[cfg(target_os = "linux")]
#[test]
fn auto_reconnait_une_iso_windows_et_ecrit_un_volume_valide() {
    if !common::outil_disponible("fsck.vfat") {
        return;
    }
    let d = Dossier::new("auto");
    let (iso, cle) = (d.file("win.iso"), d.file("cle.img"));
    fs::write(&iso, windows_iso(0x8664)).expect("écriture de l'ISO");

    let plan = bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "134217728",
        "--dry-run",
    ]);
    let text = String::from_utf8_lossy(&plan.stdout);
    assert!(plan.status.success(), "{text}");
    assert!(text.contains("extraction, 4 fichiers"), "{text}");
    assert!(text.contains("chargeur non signé"), "{text}");
    assert!(!PathBuf::from(&cle).exists());

    let done = bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "134217728",
        "--yes",
        "--progress",
        "none",
        "--json",
    ]);
    let text = String::from_utf8_lossy(&done.stdout);
    assert!(
        done.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["verify"]["files"], 4);
    assert_eq!(value["extraction"]["signer"], "unsigned");
    assert!(value["gpt_repair"].is_null());

    let volume = volume_of(&d, &cle);
    let check = Command::new("fsck.vfat")
        .args(["-n", &volume])
        .output()
        .expect("fsck.vfat");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let listing = String::from_utf8_lossy(&check.stdout);
    assert!(listing.contains("files"), "{listing}");
}

#[test]
fn une_iso_d_une_autre_architecture_reste_refusee() {
    let d = Dossier::new("arm");
    let (iso, cle) = (d.file("arm.iso"), d.file("cle.img"));
    fs::write(&iso, windows_iso(0xAA64)).expect("écriture de l'ISO");
    let auto = bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "134217728",
        "--dry-run",
    ]);
    assert_eq!(auto.status.code(), Some(6));
    let forced = bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "134217728",
        "--dry-run",
        "--mode",
        "extract",
    ]);
    assert_eq!(forced.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&forced.stderr).contains("E3006"));
}

#[test]
fn une_cible_trop_petite_est_refusee_sans_rien_ecrire() {
    let d = Dossier::new("petite");
    let (iso, cle) = (d.file("win.iso"), d.file("cle.img"));
    fs::write(&iso, windows_iso(0x8664)).expect("écriture de l'ISO");
    let out = bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "1048576",
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(out.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&out.stderr).contains("E3003"));
}
