#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use super::*;
use crate::block::MemoryDevice;

fn volume() -> Volume<'static> {
    Volume {
        first_lba: 2048,
        sectors: (64 << 20) / 512,
        label: Some("CLE"),
        volume_id: 7,
        bios_boot: false,
    }
}

fn file(path: &str, size: u64) -> NewFile {
    NewFile {
        path: path.split('/').map(str::to_owned).collect(),
        size,
    }
}

#[test]
fn un_nom_court_valide_reste_tel_quel_les_autres_recoivent_un_nom_unique() {
    let mut taken = BTreeSet::new();
    assert!(!short_name("BOOT.WIM", &mut taken).unwrap().1);
    let (a, long) = short_name("bootmgr-x.efi", &mut taken).unwrap();
    assert!(long);
    assert_eq!(&a, b"BOOTMG~1EFI");
    let (b, _) = short_name("bootmgr-y.efi", &mut taken).unwrap();
    assert_eq!(&b, b"BOOTMG~2EFI");
}

/// The BIOS second stage looks for `BOOTMGR`; Windows ISOs name it in lower case.
#[test]
fn un_nom_court_en_minuscules_garde_son_nom_en_majuscules() {
    let mut taken = BTreeSet::new();
    let (short, long) = short_name("bootmgr", &mut taken).unwrap();
    assert_eq!(&short, b"BOOTMGR    ");
    assert!(long);
    assert_eq!(
        &short_name("setup.exe", &mut taken).unwrap().0,
        b"SETUP   EXE"
    );
    let (again, _) = short_name("BootMgr", &mut taken).unwrap();
    assert_eq!(&again, b"BOOTMG~1   ");
}

/// Specification values: short name checksum, entry order and padding.
#[test]
fn les_entrees_longues_suivent_le_format() {
    let short = *b"BOOTMG~1EFI";
    let entries = long_entries("bootmgr_long_name.efi", &short).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0][0], 0x42);
    assert_eq!(entries[1][0], 0x01);
    assert!(entries.iter().all(|e| e[11] == ATTR_LONG_NAME));
    assert_eq!(entries[1][1], b'b');
    assert_eq!(entries[0][13], short_checksum(&short));
    assert!(long_entries("a:b", &short).is_err());
}

#[test]
fn les_refus_passent_avant_toute_ecriture() {
    let tiny = |f: &[NewFile]| layout(&volume(), 512, f, &[]);
    assert!(matches!(
        tiny(&[file("a", 1 << 32)]),
        Err(FormatError::TooLarge(_))
    ));
    assert!(matches!(
        tiny(&[file("a", 1 << 31)]),
        Err(FormatError::TooSmall(_))
    ));
    assert!(matches!(
        tiny(&[file("A.txt", 1), file("a.TXT", 1)]),
        Err(FormatError::Name(_))
    ));
}

#[test]
fn copie_et_relecture_detectent_un_octet_change() {
    let content: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    let files = [
        file("sources/install.swm", content.len() as u64),
        file("vide.txt", 0),
    ];
    let v = volume();
    let plan = layout(&v, 512, &files, &[vec!["boot".to_owned()]]).unwrap();
    let mut dev = MemoryDevice::new(80 << 20, 512).unwrap();
    plan.write_structures(&mut dev).unwrap();
    let mut copied = 0;
    let sha = plan
        .write_file(&mut dev, 0, &mut content.as_slice(), &mut |n| {
            copied += n;
            true
        })
        .unwrap();
    assert_eq!(copied, content.len() as u64);
    plan.write_file(&mut dev, 1, &mut [].as_slice(), &mut |_| true)
        .unwrap();
    plan.finish(&mut dev, &v).unwrap();
    plan.verify_structures(&dev, &v).unwrap();
    plan.verify_file(&dev, 0, &sha, &mut |_| true).unwrap();
    let at = plan.cluster_offset(plan.files[0].first_cluster) as usize + 50_000;
    let mut bytes = dev.as_bytes().to_vec();
    bytes[at] ^= 1;
    let bad = MemoryDevice::from_bytes(bytes, 512).unwrap();
    assert!(matches!(
        plan.verify_file(&bad, 0, &sha, &mut |_| true),
        Err(FormatError::Mismatch(_))
    ));
}

#[test]
fn un_fichier_plus_court_qu_annonce_est_une_erreur() {
    let v = volume();
    let plan = layout(&v, 512, &[file("a", 1000)], &[]).unwrap();
    let mut dev = MemoryDevice::new(80 << 20, 512).unwrap();
    plan.write_structures(&mut dev).unwrap();
    assert!(matches!(
        plan.write_file(&mut dev, 0, &mut [0u8; 10].as_slice(), &mut |_| true),
        Err(FormatError::Source(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof
    ));
}
