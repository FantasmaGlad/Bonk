// `fsck.exfat`, the only independent judge here, exists in CI on Linux only.
#![cfg(target_os = "linux")]
#![allow(clippy::expect_used)]

use std::process::Command;

use bonk_core::MemoryDevice;
use bonk_core::format::NewFile;
use bonk_core::format::exfat::Volume;
use bonk_core::format::exfat_tree::layout;

mod common;

const MIB: u64 = 1 << 20;

fn file(path: &str, size: u64) -> NewFile {
    NewFile {
        path: path.split('/').map(str::to_owned).collect(),
        size,
    }
}

/// A tree written by Bonk, read back by the official tool: long and accented names, nested directories, an
/// empty file, a file over several clusters.
#[test]
fn une_arborescence_exfat_est_acceptee_par_fsck_exfat() {
    let long = "Un nom de fichier assez long pour plusieurs entrées de nom, été 2026.iso";
    let content: Vec<u8> = (0..300_000u32).map(|i| (i * 31) as u8).collect();
    let files = [
        file(&format!("isos/linux/{long}"), content.len() as u64),
        file("isos/vide.txt", 0),
        file("LISEZ-MOI.txt", 5),
    ];
    let volume = Volume {
        first_lba: 0,
        sectors: 64 * MIB / 512,
        label: Some("BonkVault"),
        volume_id: 0xB0B0,
    };
    let plan = layout(&volume, 512, &files, &[vec!["boot".to_owned()]]).expect("layout");
    let mut dev = MemoryDevice::new(64 * MIB, 512).expect("support");
    plan.write_structures(&mut dev).expect("structures");
    let mut written = Vec::new();
    for (index, bytes) in [content.as_slice(), &[], b"bonk\n"].into_iter().enumerate() {
        let mut reader = bytes;
        written.push(
            plan.write_file(&mut dev, index, &mut reader, &mut |_| true)
                .expect("copie"),
        );
    }
    plan.finish(&mut dev, &volume).expect("démarrage");
    plan.verify_structures(&dev, &volume).expect("relecture");
    for (index, sum) in written.iter().enumerate() {
        plan.verify_file(&dev, index, sum, &mut |_| true)
            .expect("relecture du fichier");
    }

    // Bonk's own read back needs no tool: only the official check depends on it.
    if !common::outil_disponible("fsck.exfat") {
        return;
    }
    let path = std::env::temp_dir().join(format!("bonk-exfat-arbo-{}.img", std::process::id()));
    std::fs::write(&path, dev.as_bytes()).expect("image");
    let fsck = Command::new("fsck.exfat")
        .arg("-n")
        .arg("-v")
        .arg(&path)
        .output()
        .expect("fsck.exfat");
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&fsck.stdout);
    assert_eq!(
        fsck.status.code(),
        Some(0),
        "{text}{}",
        String::from_utf8_lossy(&fsck.stderr)
    );
    assert!(text.contains("directories 4, files 3"), "{text}");
}

fn fsck(image: &[u8], label: &str) -> String {
    let path = std::env::temp_dir().join(format!("bonk-exfat-{label}-{}.img", std::process::id()));
    std::fs::write(&path, image).expect("image");
    let fsck = Command::new("fsck.exfat")
        .arg("-n")
        .arg("-v")
        .arg(&path)
        .output()
        .expect("fsck.exfat");
    let _ = std::fs::remove_file(&path);
    let text = String::from_utf8_lossy(&fsck.stdout).into_owned();
    assert_eq!(
        fsck.status.code(),
        Some(0),
        "{text}{}",
        String::from_utf8_lossy(&fsck.stderr)
    );
    text
}

/// Added and removed in a volume made by Bonk then in one made by mkfs.exfat: the official tool finds both
/// clean, files and free space included.
#[test]
fn un_ajout_et_un_retrait_sont_acceptes_par_fsck_exfat() {
    use bonk_core::format::exfat_edit::{Span, add_file, remove_file};

    if !common::outil_disponible("fsck.exfat") || !common::outil_disponible("mkfs.exfat") {
        return;
    }
    let made_by_bonk = {
        let mut dev = MemoryDevice::new(64 * MIB, 512).expect("support");
        bonk_core::format::exfat::write(
            &mut dev,
            &Volume {
                first_lba: 0,
                sectors: 64 * MIB / 512,
                label: Some("BonkVault"),
                volume_id: 0xB0B0,
            },
        )
        .expect("volume");
        dev.as_bytes().to_vec()
    };
    let made_by_mkfs = {
        let path = std::env::temp_dir().join(format!("bonk-mkfs-exfat-{}.img", std::process::id()));
        std::fs::write(&path, vec![0u8; 64 * MIB as usize]).expect("image");
        let made = Command::new("mkfs.exfat")
            .args(["-L", "BonkVault"])
            .arg(&path)
            .output()
            .expect("mkfs.exfat");
        assert!(
            made.status.success(),
            "{}",
            String::from_utf8_lossy(&made.stderr)
        );
        let bytes = std::fs::read(&path).expect("relecture");
        let _ = std::fs::remove_file(&path);
        bytes
    };
    for (label, image) in [("bonk", made_by_bonk), ("mkfs", made_by_mkfs)] {
        let mut dev = MemoryDevice::from_bytes(image, 512).expect("support");
        let span = Span {
            first_lba: 0,
            sectors: 64 * MIB / 512,
        };
        let big: Vec<u8> = (0..5_000_000u32).map(|i| (i * 13) as u8).collect();
        for (name, data) in [
            ("archlinux-x86_64.iso", big.as_slice()),
            (
                "Un nom assez long pour plusieurs entrées, été.iso",
                b"bonk\n".as_slice(),
            ),
            ("vide.iso", [].as_slice()),
        ] {
            let mut reader = data;
            add_file(
                &mut dev,
                span,
                name,
                data.len() as u64,
                false,
                &mut reader,
                &mut |_| true,
            )
            .expect("ajout");
        }
        let text = fsck(dev.as_bytes(), label);
        assert!(text.contains("files 3"), "{label}: {text}");
        remove_file(&mut dev, span, "archlinux-x86_64.iso").expect("retrait");
        let text = fsck(dev.as_bytes(), label);
        assert!(text.contains("files 2"), "{label}: {text}");
    }
}
