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

/// Une arborescence écrite par Bonk, relue par l'outil officiel : noms longs et accentués, dossiers
/// imbriqués, fichier vide, fichier sur plusieurs clusters.
#[test]
fn une_arborescence_exfat_est_acceptee_par_fsck_exfat() {
    if !common::outil_disponible("fsck.exfat") {
        return;
    }
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
