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
    windows_iso_with("efi/boot/bootx64.efi", machine)
}

fn windows_iso_with(loader_path: &str, machine: u16) -> Vec<u8> {
    let content: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    build(&[
        (loader_path, &loader(machine)),
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
    assert!(text.contains("extraction, 4 files"), "{text}");
    assert!(text.contains("unsigned loader"), "{text}");
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
    assert_eq!(
        value["extraction"]["architectures"],
        serde_json::json!(["x86_64"])
    );
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

fn plan(d: &Dossier, iso_bytes: Vec<u8>) -> Output {
    let (iso, cle) = (d.file("win.iso"), d.file("cle.img"));
    fs::write(&iso, iso_bytes).expect("écriture de l'ISO");
    bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "134217728",
        "--dry-run",
        "--json",
    ])
}

#[test]
fn une_iso_arm64_ou_x86_est_ecrite_avec_son_architecture() {
    for (label, path, machine, arch) in [
        ("arm64", "efi/boot/bootaa64.efi", 0xAA64, "arm64"),
        ("x86", "efi/boot/bootia32.efi", 0x14C, "x86"),
    ] {
        let d = Dossier::new(label);
        let out = plan(&d, windows_iso_with(path, machine));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{label}: {text}");
        let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(
            value["extraction"]["architectures"],
            serde_json::json!([arch]),
            "{text}"
        );
    }
}

#[test]
fn un_chargeur_qui_ment_sur_sa_puce_reste_refuse() {
    let d = Dossier::new("ment");
    let iso = d.file("ment.iso");
    fs::write(&iso, windows_iso_with("efi/boot/bootaa64.efi", 0x8664)).expect("écriture");
    let forced = bonk(&[
        "write",
        "--simulate",
        &d.file("cle.img"),
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

fn windows_iso_bios(loader_path: &str, machine: u16) -> Vec<u8> {
    let bootmgr: Vec<u8> = (0..50_000u32).map(|i| (i * 7) as u8).collect();
    let content: Vec<u8> = (0..200_000u32).map(|i| i as u8).collect();
    build(&[
        (loader_path, &loader(machine)),
        ("bootmgr", &bootmgr),
        ("sources/install.wim", &content),
    ])
}

#[test]
fn une_table_mbr_porte_le_code_bios_et_bootmgr_garde_son_nom_court() {
    let d = Dossier::new("bios");
    let (iso, cle) = (d.file("win.iso"), d.file("cle.img"));
    fs::write(&iso, windows_iso_bios("efi/boot/bootx64.efi", 0x8664)).expect("écriture de l'ISO");
    let write = |table: &str, extra: &[&str]| {
        let mut args = vec![
            "write",
            "--simulate",
            &cle,
            &iso,
            "--size",
            "134217728",
            "--table",
            table,
        ];
        args.extend_from_slice(extra);
        bonk(&args)
    };

    let gpt = write("gpt", &["--dry-run"]);
    let text = String::from_utf8_lossy(&gpt.stdout);
    assert!(text.contains("BIOS boot: no, GPT table"), "{text}");

    let done = write("mbr", &["--yes", "--progress", "none", "--json"]);
    let text = String::from_utf8_lossy(&done.stdout);
    assert!(
        done.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(
        value["extraction"]["firmware"],
        serde_json::json!(["uefi", "bios"])
    );

    let bytes = fs::read(&cle).expect("lecture de la clé");
    assert_eq!(&bytes[..440], &bonk_core::boot::MBR[..]);
    assert_eq!(bytes[446], 0x80);
    let volume = &bytes[1 << 20..];
    assert_eq!(&volume[90..510], &bonk_core::boot::VBR[90..510]);
    let stage2 = bonk_core::boot::STAGE2_SECTOR as usize * 512;
    assert_eq!(&volume[stage2..stage2 + 4096], &bonk_core::boot::STAGE2[..]);
    let short = volume
        .as_chunks::<32>()
        .0
        .iter()
        .any(|entry| &entry[..11] == b"BOOTMGR    " && entry[11] & 0x18 == 0);
    assert!(short, "entrée courte BOOTMGR absente");
}

#[test]
fn une_iso_sans_bootmgr_pour_x86_reste_en_uefi_seul() {
    let d = Dossier::new("arm-bios");
    let (iso, cle) = (d.file("win.iso"), d.file("cle.img"));
    fs::write(&iso, windows_iso_bios("efi/boot/bootaa64.efi", 0xAA64)).expect("écriture");
    let out = bonk(&[
        "write",
        "--simulate",
        &cle,
        &iso,
        "--size",
        "134217728",
        "--table",
        "mbr",
        "--dry-run",
        "--json",
    ]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["extraction"]["firmware"], serde_json::json!(["uefi"]));
}

#[test]
fn une_cle_windows_est_en_mbr_par_defaut() {
    let d = Dossier::new("defaut");
    let out = plan(&d, windows_iso_bios("efi/boot/bootx64.efi", 0x8664));
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    let value: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(value["extraction"]["table"], "mbr");
    assert_eq!(
        value["extraction"]["firmware"],
        serde_json::json!(["uefi", "bios"])
    );
}
