#![allow(clippy::unwrap_used)]

use bonk_core::MemoryDevice;

use super::*;

fn ids() -> Ids {
    Ids::from_seed(b"essai")
}

#[test]
fn fat32_refuse_sur_secteurs_de_4_kio_ne_laisse_aucune_table() {
    let mut device = MemoryDevice::new(100 << 20, 4096).unwrap();
    let result = write_volume(
        &mut device,
        TableKind::Gpt,
        Filesystem::Fat32,
        None,
        &ids(),
        &|| false,
    );
    assert!(result.is_err());
    assert!(device.as_bytes().iter().all(|b| *b == 0));
}

#[test]
fn l_annulation_arrete_le_formatage_entre_ses_etapes() {
    let mut device = MemoryDevice::new(100 << 20, 512).unwrap();
    let result = write_volume(
        &mut device,
        TableKind::Gpt,
        Filesystem::Fat32,
        None,
        &ids(),
        &|| true,
    );
    assert_eq!(result.err().map(|e| e.code), Some(Code::Cancelled));
    let volume = 1 << 20;
    assert!(
        device.as_bytes()[volume..volume + 512]
            .iter()
            .all(|b| *b == 0)
    );
}
