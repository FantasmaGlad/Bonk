#![allow(clippy::unwrap_used)]

use bonk_core::format::Ids;
use bonk_core::format::table::TableKind;
use bonk_core::vault::{self};

use crate::errors::Code;

use bonk_core::MemoryDevice;

use super::*;

/// Cancel is seen between steps: no volume after a cancelled table.
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
