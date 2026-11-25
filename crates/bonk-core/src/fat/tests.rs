#![allow(clippy::expect_used)]

use super::testing::{fat32, small};
use super::*;

const LOADER: &str = "EFI/BOOT/BOOTX64.EFI";

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 251) as u8).collect()
}

#[test]
fn chaque_variante_de_fat_rend_le_fichier_entier() {
    let data = pattern(5000);
    let files: &[(&str, &[u8])] = &[("EFI/BOOT/GRUBX64.EFI", b"grub"), (LOADER, &data)];
    let fat12 = small(Kind::Fat12, files).image;
    let fat16 = small(Kind::Fat16, files).image;
    let fat32 = fat32(files);
    for (image, kind) in [
        (&fat12, Kind::Fat12),
        (&fat16, Kind::Fat16),
        (&fat32, Kind::Fat32),
    ] {
        let fat = Fat::open(image.as_slice(), 0).expect("FAT");
        assert_eq!(fat.kind(), kind);
        assert_eq!(
            fat.read(LOADER, 1 << 20).as_deref(),
            Some(data.as_slice()),
            "{kind:?}"
        );
        assert_eq!(
            fat.read("efi/boot/grubx64.efi", 1 << 20).as_deref(),
            Some(&b"grub"[..])
        );
    }
}

#[test]
fn un_volume_place_plus_loin_se_lit_depuis_son_debut() {
    let mut image = vec![0u8; 4096];
    image.extend(small(Kind::Fat12, &[(LOADER, b"loader")]).image);
    let fat = Fat::open(image.as_slice(), 4096).expect("FAT");
    assert_eq!(fat.read(LOADER, 64).as_deref(), Some(&b"loader"[..]));
}

#[test]
fn un_nom_long_ecrit_en_minuscules_se_trouve_par_son_nom_court() {
    let image = fat32(&[("efi/boot/bootx64.efi", b"loader")]);
    let fat = Fat::open(image.as_slice(), 0).expect("FAT");
    assert_eq!(fat.read(LOADER, 64).as_deref(), Some(&b"loader"[..]));
}

#[test]
fn un_fichier_absent_trop_grand_ou_mal_nomme_ne_rend_rien() {
    let image = small(Kind::Fat16, &[(LOADER, b"loader"), ("EFI/EMPTY.TXT", b"")]).image;
    let fat = Fat::open(image.as_slice(), 0).expect("FAT");
    assert_eq!(fat.read("EFI/EMPTY.TXT", 64).as_deref(), Some(&b""[..]));
    for (path, limit) in [
        ("EFI/BOOT/BOOTAA64.EFI", 64),
        (LOADER, 5),
        ("EFI/BOOT", 64),
        ("EFI/EMPTY.TXT/X", 64),
        ("EFI/BOOT/BOOTX64.EFI.OLD", 64),
        ("", 64),
    ] {
        assert_eq!(fat.read(path, limit), None, "{path}");
    }
}

#[test]
fn une_chaine_qui_boucle_ou_sort_du_volume_est_refusee() {
    let data = pattern(2000);
    for next in [0x004, 0x003, 0xFF7, 0x000, 0x0FF0] {
        let mut fat = small(Kind::Fat12, &[(LOADER, &data)]);
        fat.set_fat(5, next);
        let image = fat.image;
        let reader = Fat::open(image.as_slice(), 0).expect("FAT");
        assert_eq!(reader.read(LOADER, 1 << 20), None, "{next:#x}");
    }
}

#[test]
fn ce_qui_n_est_pas_un_fat_n_ouvre_pas() {
    let good = small(Kind::Fat12, &[(LOADER, b"loader")]);
    let mut unsigned = good.image.clone();
    unsigned[510] = 0;
    let mut no_cluster = good.image.clone();
    no_cluster[13] = 3;
    for image in [vec![0u8; 512], unsigned, no_cluster] {
        assert!(Fat::open(image.as_slice(), 0).is_none());
    }
    let cut = &good.image[..good.fat_offset() * 2];
    let fat = Fat::open(cut, 0).expect("boot sector alone");
    assert_eq!(fat.read(LOADER, 64), None);
}

#[test]
fn un_nom_long_se_lit_sur_plusieurs_entrees_et_sans_casse() {
    let image = fat32(&[
        ("bonk/families.json", b"[]"),
        ("bonk/un-nom-bien-plus-long-que-treize.json", b"long"),
    ]);
    let fat = Fat::open(image.as_slice(), 0).expect("FAT");
    assert_eq!(
        fat.read("bonk/families.json", 64).as_deref(),
        Some(&b"[]"[..])
    );
    assert_eq!(
        fat.read("BONK/Families.JSON", 64).as_deref(),
        Some(&b"[]"[..])
    );
    assert_eq!(
        fat.read("bonk/un-nom-bien-plus-long-que-treize.json", 64)
            .as_deref(),
        Some(&b"long"[..])
    );
    assert_eq!(fat.read("bonk/un-nom-bien-plus-long-que-treize", 64), None);
}
