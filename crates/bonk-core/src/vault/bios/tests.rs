use super::*;
use crate::block::MemoryDevice;
use crate::format::Ids;
use crate::partition::read_range;
use crate::vault::{VaultBoot, plan, write_table};

fn grub(core_sectors: usize) -> BiosGrub {
    let mut boot = vec![0xEB; SECTOR];
    boot[510..].copy_from_slice(&[0x55, 0xAA]);
    let mut core = vec![0xC0; core_sectors * SECTOR - 100];
    let list = SECTOR - LIST_SIZE;
    core[list..list + 8].copy_from_slice(&2u64.to_le_bytes());
    core[list + 8..list + 10].copy_from_slice(&7u16.to_le_bytes());
    core[list - LIST_SIZE + 8..list - LIST_SIZE + 10].copy_from_slice(&3u16.to_le_bytes());
    core[list - 2 * LIST_SIZE..list - LIST_SIZE].fill(0);
    BiosGrub { boot, core }
}

fn sector0() -> [u8; SECTOR] {
    let mut sector = [0u8; SECTOR];
    sector[0x1B8..0x1BC].copy_from_slice(&0x1234_5678u32.to_le_bytes());
    sector[0x1BE + 4] = 0xEE;
    sector[510..].copy_from_slice(&[0x55, 0xAA]);
    sector
}

#[test]
fn boot_img_nomme_core_img_et_garde_la_table() -> Result<(), FormatError> {
    let (mbr, core) = bios_sectors(&grub(3), &sector0(), 34, 2048)?;
    assert_eq!(&mbr[KERNEL_SECTOR..KERNEL_SECTOR + 8], &34u64.to_le_bytes());
    assert_eq!(mbr[BOOT_DRIVE], 0xFF);
    assert_eq!(&mbr[DRIVE_CHECK..DRIVE_CHECK + 2], &[0x90, 0x90]);
    assert_eq!(&mbr[TABLE], &sector0()[TABLE]);
    assert_eq!(&mbr[BPB], &sector0()[BPB]);
    assert_eq!(&mbr[510..], &[0x55, 0xAA]);
    assert_eq!(mbr[0], 0xEB);
    assert_eq!(core.len(), 3 * SECTOR);
    let list = SECTOR - LIST_SIZE;
    assert_eq!(&core[list..list + 8], &35u64.to_le_bytes());
    assert_eq!(&core[list + 8..list + 10], &2u16.to_le_bytes());
    assert_eq!(&core[list + 10..list + 12], &0x820u16.to_le_bytes());
    assert!(
        core[list - LIST_SIZE..list].iter().all(|b| *b == 0),
        "ancienne liste effacée"
    );
    assert!(core[3 * SECTOR - 100..].iter().all(|b| *b == 0));
    Ok(())
}

#[test]
fn un_grub_qui_ne_tient_pas_ou_n_est_pas_celui_de_grub_est_refuse() {
    let mut no_signature = grub(3);
    no_signature.boot[511] = 0;
    let short = BiosGrub {
        core: vec![0; 100],
        ..grub(3)
    };
    let mut no_terminator = grub(2);
    for at in (0..=SECTOR - LIST_SIZE).rev().step_by(LIST_SIZE) {
        no_terminator.core[at + 8] = 1;
    }
    let cases = [
        (no_signature, 34, 2048),
        (short, 34, 2048),
        (grub(3), 0, 2048),
        (grub(3), 34, 36),
        (no_terminator, 1, 2048),
    ];
    for (grub, first, end) in cases {
        assert!(matches!(
            bios_sectors(&grub, &sector0(), first, end),
            Err(FormatError::Unsupported(_))
        ));
    }
}

fn vault(block: u32, table: TableKind) -> Result<(MemoryDevice, VaultPlan), FormatError> {
    let mut dev = MemoryDevice::new(256 << 20, block)?;
    let plan = plan(dev.size(), block, table)?;
    write_table(&mut dev, &plan, &Ids::from_seed(b"bios"))?;
    Ok((dev, plan))
}

#[test]
fn le_coffre_demarre_en_bios_en_mbr_comme_en_gpt() -> Result<(), FormatError> {
    for (table, first, active) in [(TableKind::Mbr, 1u64, ACTIVE), (TableKind::Gpt, 34, 0)] {
        let (mut dev, plan) = vault(512, table)?;
        let before = read_range(&dev, 0, 512)?;
        assert!(!bios_matches(&dev, &plan, &grub(3))?);
        write_bios(&mut dev, &plan, &grub(3))?;
        assert!(bios_matches(&dev, &plan, &grub(3))?);
        let mbr = read_range(&dev, 0, 512)?;
        assert_eq!(&mbr[..3], &[0xEB; 3]);
        assert_eq!(&mbr[KERNEL_SECTOR..KERNEL_SECTOR + 8], &first.to_le_bytes());
        assert_eq!(mbr[FIRST_ENTRY], active, "{table:?}");
        assert_eq!(
            &mbr[TABLE.start..FIRST_ENTRY],
            &before[TABLE.start..FIRST_ENTRY]
        );
        assert_eq!(&mbr[FIRST_ENTRY + 1..], &before[FIRST_ENTRY + 1..]);
        let core = read_range(&dev, first * 512, 512)?;
        let list = SECTOR - LIST_SIZE;
        assert_eq!(&core[list..list + 8], &(first + 1).to_le_bytes());
        let mut newer = grub(3);
        newer.core[700] ^= 1;
        assert!(!bios_matches(&dev, &plan, &newer)?);
    }
    Ok(())
}

#[test]
fn un_disque_a_secteurs_de_4_kio_demarre_en_uefi_seulement() {
    let boot = VaultBoot {
        files: Vec::new(),
        bios: Some(grub(3)),
    };
    assert!(boot.bios_on(512).is_some());
    assert!(boot.bios_on(4096).is_none());
}

#[test]
fn un_core_img_qui_deborde_sur_le_coffre_est_refuse() -> Result<(), FormatError> {
    let (mut dev, plan) = vault(512, TableKind::Mbr)?;
    let before = read_range(&dev, 0, 512)?;
    let large = grub(2048);
    assert!(matches!(
        write_bios(&mut dev, &plan, &large),
        Err(FormatError::Unsupported(_))
    ));
    assert_eq!(read_range(&dev, 0, 512)?, before);
    Ok(())
}
