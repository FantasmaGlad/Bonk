use std::io::Cursor;

use super::*;

#[test]
fn fichiers_trouves_par_leur_nom_rock_ridge() -> io::Result<()> {
    let image = testing::image(
        "Fedora-WS-Live-44",
        &[
            ("boot/x86_64/loader/linux", 18_479_464),
            ("boot/x86_64/loader/initrd", 1000),
            ("EFI/BOOT/grub.cfg", 111),
        ],
    );
    let mut iso = Iso::open(Cursor::new(image))?;
    assert_eq!(iso.label(), "Fedora-WS-Live-44");
    assert_eq!(
        iso.file_size("/boot/x86_64/loader/linux")?,
        Some(18_479_464)
    );
    assert_eq!(iso.file_size("EFI/BOOT/GRUB.CFG")?, Some(111));
    assert_eq!(iso.file_size("boot/x86_64/loader")?, None);
    assert_eq!(iso.file_size("boot/x86_64/loader/absent")?, None);
    assert_eq!(iso.file_size("boot/x86_64/loader/linux/plus")?, None);
    Ok(())
}

#[test]
fn un_motif_trouve_les_fichiers_de_son_dossier() -> io::Result<()> {
    let image = testing::image(
        "Q4OS 6 Andromeda",
        &[
            ("live/vmlinuz-6.12.118+deb13-amd64", 1),
            ("live/initrd.img-6.12.118+deb13-amd64", 1),
            ("live/filesystem.squashfs", 1),
        ],
    );
    let mut iso = Iso::open(Cursor::new(image))?;
    let cas = [
        ("live/vmlinuz-*", vec!["live/vmlinuz-6.12.118+deb13-amd64"]),
        (
            "LIVE/INITRD.IMG-*",
            vec!["LIVE/initrd.img-6.12.118+deb13-amd64"],
        ),
        ("live/*.squashfs", vec!["live/filesystem.squashfs"]),
        ("live/vmlinuz", vec![]),
        ("absent/vmlinuz-*", vec![]),
        ("live/vmlinuz-6.12.118+deb13-amd64/*", vec![]),
    ];
    for (motif, attendu) in cas {
        assert_eq!(iso.matching(motif)?, attendu, "{motif}");
    }
    Ok(())
}

#[test]
fn une_image_qui_n_est_pas_iso_est_refusee() {
    assert!(Iso::open(Cursor::new(vec![0u8; 40 * 2048])).is_err());
    assert!(Iso::open(Cursor::new(vec![0u8; 100])).is_err());
}

#[test]
fn un_repertoire_demesure_est_refuse_sans_allocation() -> io::Result<()> {
    let mut image = testing::image("X", &[("a/b", 1)]);
    let at = (FIRST_DESCRIPTOR * SECTOR) as usize + 156 + 10;
    image[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    let mut iso = Iso::open(Cursor::new(image))?;
    assert!(iso.file_size("a/b").is_err());
    Ok(())
}

#[test]
fn nom_iso_sans_rock_ridge() {
    let mut record = vec![0u8; 33];
    record.extend(b"VMLINUZ.;1");
    record[32] = 10;
    assert_eq!(name_of(&record).as_deref(), Some("VMLINUZ"));
}
