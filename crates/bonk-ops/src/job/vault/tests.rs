#![allow(clippy::unwrap_used)]

use bonk_core::format::Ids;
use bonk_core::format::table::TableKind;
use bonk_core::vault;

use crate::errors::Code;

use bonk_core::MemoryDevice;

use super::*;

#[test]
fn l_annulation_arrete_avant_les_volumes() {
    let mut device = MemoryDevice::new(256 << 20, 512).unwrap();
    let result = write_vault(
        &mut device,
        TableKind::Mbr,
        &Ids::from_seed(b"essai"),
        &[],
        &|| true,
    );
    assert_eq!(result.err().map(|e| e.code), Some(Code::Cancelled));
    assert_eq!(vault::detect(&device).unwrap(), None);
}

#[test]
fn un_disque_trop_petit_ne_recoit_rien() {
    let mut device = MemoryDevice::new(100 << 20, 512).unwrap();
    let result = write_vault(
        &mut device,
        TableKind::Mbr,
        &Ids::from_seed(b"essai"),
        &[],
        &|| false,
    );
    assert_eq!(result.err().map(|e| e.code), Some(Code::FormatFailed));
    assert!(device.as_bytes().iter().all(|b| *b == 0));
}

#[test]
fn les_espaces_d_un_nom_d_iso_deviennent_des_soulignes() {
    use super::edit::vault_name;
    let path = std::path::Path::new("/tmp/Ubuntu 26.04 desktop.iso");
    assert_eq!(
        vault_name(path, None).ok().as_deref(),
        Some("Ubuntu_26.04_desktop.iso")
    );
    assert_eq!(
        vault_name(path, Some("ma copie.ISO")).ok().as_deref(),
        Some("ma_copie.ISO")
    );
    let refus = vault_name(path, Some("image.img")).err();
    assert_eq!(refus.map(|e| e.code), Some(Code::IsoNotForVault));
}
