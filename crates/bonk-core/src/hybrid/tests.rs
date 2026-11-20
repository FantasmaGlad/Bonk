#![allow(clippy::expect_used)]

use super::*;

const SECTOR: usize = 2048;

/// An ISO head with a boot record pointing to a catalog at block 20 holding `entries`: (platform, block,
/// 512-byte count); the first is the default entry.
fn head(entries: &[(u8, u32, u16)]) -> Vec<u8> {
    let mut head = vec![0u8; 32 * SECTOR];
    let record = &mut head[17 * SECTOR..18 * SECTOR];
    record[1..6].copy_from_slice(b"CD001");
    record[6] = 1;
    record[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
    record[0x47..0x4B].copy_from_slice(&20u32.to_le_bytes());
    let catalog = &mut head[20 * SECTOR..21 * SECTOR];
    let entry = |slot: &mut [u8], block: u32, count: u16| {
        slot[0] = 0x88;
        slot[6..8].copy_from_slice(&count.to_le_bytes());
        slot[8..12].copy_from_slice(&block.to_le_bytes());
    };
    let (platform, block, count) = entries[0];
    catalog[0] = 1;
    catalog[1] = platform;
    catalog[30] = 0x55;
    catalog[31] = 0xAA;
    entry(&mut catalog[32..64], block, count);
    for (index, (platform, block, count)) in entries[1..].iter().enumerate() {
        let at = 64 + index * 64;
        catalog[at] = if index + 2 == entries.len() {
            0x91
        } else {
            0x90
        };
        catalog[at + 1] = *platform;
        catalog[at + 2] = 1;
        entry(&mut catalog[at + 32..at + 64], *block, *count);
    }
    head
}

/// As the published NixOS 25.11 aarch64 ISO: EFI image at block 55, 6144 sectors (`xorriso -report_el_torito`).
#[test]
fn l_image_efi_devient_la_partition_efi_du_mbr() {
    let mbr = hybrid_mbr(&head(&[(PLATFORM_EFI, 55, 6144)])).expect("hybride");
    assert_eq!(mbr[446 + 4], 0xEF);
    assert_eq!(&mbr[446 + 8..446 + 12], &220u32.to_le_bytes());
    assert_eq!(&mbr[446 + 12..446 + 16], &6144u32.to_le_bytes());
    assert_eq!(&mbr[510..], &[0x55, 0xAA]);
    assert!(mbr[462..510].iter().all(|b| *b == 0), "une seule partition");
}

#[test]
fn l_entree_efi_d_une_section_est_trouvee_apres_l_entree_bios() {
    let mbr = hybrid_mbr(&head(&[(0, 30, 4), (PLATFORM_EFI, 39, 18140)])).expect("hybride");
    assert_eq!(&mbr[446 + 8..446 + 12], &156u32.to_le_bytes());
}

#[test]
fn ce_qui_ne_se_rend_pas_hybride_est_refuse_en_le_disant() {
    let mut occupied = head(&[(PLATFORM_EFI, 55, 6144)]);
    occupied[0] = 0xEB;
    // (head, reason)
    let cases = [
        (occupied, NotHybrid::SystemAreaUsed),
        (head(&[(0, 30, 4)]), NotHybrid::NoEfiImage),
        (vec![0u8; 32 * SECTOR], NotHybrid::NoEfiImage),
        (
            head(&[(PLATFORM_EFI, 39, 100), (PLATFORM_EFI, 60, 100)]),
            NotHybrid::SeveralEfiImages,
        ),
        (
            head(&[(PLATFORM_EFI, u32::MAX, 100)]),
            NotHybrid::ImageOutOfReach,
        ),
    ];
    for (head, reason) in cases {
        assert_eq!(hybrid_mbr(&head), Err(reason));
    }
}

#[test]
fn l_iso_dvd_de_banc_se_rend_hybride() {
    let iso = testing::dvd_iso(128);
    let mbr = hybrid_mbr(&iso).expect("hybride");
    assert_eq!(
        &mbr[446 + 8..446 + 12],
        &(testing::EFI_BLOCK * 4).to_le_bytes()
    );
    assert_eq!(
        crate::analysis::analyze(&iso, Some(iso.len() as u64)).kind,
        crate::analysis::ImageKind::PlainIso
    );
}
