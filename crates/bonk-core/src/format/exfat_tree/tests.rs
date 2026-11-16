#![allow(clippy::unwrap_used)]

use super::*;
use crate::block::MemoryDevice;

fn volume() -> Volume<'static> {
    Volume {
        first_lba: 2048,
        sectors: (64 << 20) / 512,
        label: Some("CLE"),
        volume_id: 7,
    }
}

fn file(path: &str, size: u64) -> NewFile {
    NewFile {
        path: path.split('/').map(str::to_owned).collect(),
        size,
    }
}

#[test]
fn deux_noms_egaux_en_majuscules_sont_refuses() {
    let v = volume();
    let refused = layout(&v, 512, &[file("été.iso", 1), file("ÉTÉ.ISO", 1)], &[]);
    assert!(matches!(refused, Err(FormatError::Name(_))));
    assert!(layout(&v, 512, &[file("a:b", 1)], &[]).is_err());
    assert!(layout(&v, 512, &[file("été.iso", 1), file("ete.iso", 1)], &[]).is_ok());
}

/// Checksum and name hash recomputed by hand from the specification.
#[test]
fn jeu_d_entrees_selon_la_specification() {
    let upcase = upcase_map();
    let units: Vec<u16> = "Debian-13.iso".encode_utf16().collect();
    let placed = Placed {
        first_cluster: 9,
        clusters: 3,
    };
    let set = entry_set(&units, &upcase, false, placed, 10_000);
    assert_eq!(set.len(), 3 * ENTRY);
    assert_eq!((set[0], set[1], set[32], set[64]), (0x85, 2, 0xC0, 0xC1));
    assert_eq!(set[33], ALLOCATION_POSSIBLE | NO_FAT_CHAIN);
    let mut sum: u16 = 0;
    for (i, byte) in set.iter().enumerate() {
        if i == 2 || i == 3 {
            continue;
        }
        sum = (if sum & 1 == 1 { 0x8000 } else { 0 }) + (sum >> 1) + u16::from(*byte);
    }
    assert_eq!(u16::from_le_bytes([set[2], set[3]]), sum);
    // Name hash of `DEBIAN-13.ISO`, computed apart from this code.
    assert_eq!(u16::from_le_bytes([set[36], set[37]]), 0x0205);
}

#[test]
fn copie_et_relecture_detectent_un_octet_change() {
    let content: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
    let files = [
        file("isos/debian/debian.iso", content.len() as u64),
        file("vide.txt", 0),
    ];
    let v = volume();
    let plan = layout(&v, 512, &files, &[vec!["boot".to_owned()]]).unwrap();
    let mut dev = MemoryDevice::new(80 << 20, 512).unwrap();
    plan.write_structures(&mut dev).unwrap();
    let sha = plan
        .write_file(&mut dev, 0, &mut content.as_slice(), &mut |_| true)
        .unwrap();
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
fn un_volume_trop_petit_pour_les_fichiers_est_refuse() {
    let v = volume();
    assert!(matches!(
        layout(&v, 512, &[file("gros.iso", 1 << 30)], &[]),
        Err(FormatError::TooSmall(_))
    ));
}

#[test]
fn un_fichier_de_plus_de_4_gio_est_accepte() {
    let v = Volume {
        sectors: (8u64 << 30) / 512,
        ..volume()
    };
    let plan = layout(&v, 512, &[file("windows.iso", 5 << 30)], &[]).unwrap();
    assert_eq!(plan.data_bytes(), 5 << 30);
}
