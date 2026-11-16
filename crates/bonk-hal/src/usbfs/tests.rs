use super::*;

#[test]
fn serie_lue_dans_un_descripteur_de_chaine() {
    let mut descriptor = vec![0, 3];
    for unit in "AA01 ".encode_utf16() {
        descriptor.extend(unit.to_le_bytes());
    }
    descriptor[0] = descriptor.len() as u8;
    assert_eq!(string_of(&descriptor).as_deref(), Some("AA01"));
    // The length byte counts, not the buffer: bytes beyond it are left out.
    let mut padded = descriptor.clone();
    padded.extend([b'Z', 0]);
    assert_eq!(string_of(&padded).as_deref(), Some("AA01"));
    assert_eq!(string_of(&[2, 3]), None);
    assert_eq!(string_of(&[4, 2, b'A', 0]), None);
    assert_eq!(string_of(&[]), None);
    // An unpaired surrogate is not text.
    assert_eq!(string_of(&[4, 3, 0x00, 0xD8]), None);
}

/// Typical real key: device, configuration, storage interface, two endpoints.
fn stockage() -> Vec<u8> {
    let mut bytes = vec![18, 1];
    bytes.extend([0; 16]);
    bytes.extend([9, 2, 32, 0, 1, 1, 0, 0x80, 50]);
    bytes.extend([
        9,
        4,
        3,
        0,
        2,
        CLASS_STORAGE,
        SUBCLASS_SCSI,
        PROTOCOL_BULK_ONLY,
        0,
    ]);
    bytes.extend([7, 5, 0x81, 2, 0, 2, 0]);
    bytes.extend([7, 5, 0x02, 2, 0, 2, 0]);
    bytes
}

#[test]
fn trouve_l_interface_et_ses_deux_points_de_terminaison() {
    assert_eq!(
        find_storage_interface(&stockage()),
        Some(StorageInterface {
            number: 3,
            endpoint_in: 0x81,
            endpoint_out: 0x02
        })
    );
}

#[test]
fn ignore_une_autre_classe_et_prend_le_stockage_qui_suit() {
    let mut bytes = vec![9, 4, 0, 0, 1, 3, 1, 1, 0];
    bytes.extend([7, 5, 0x83, 3, 8, 0, 10]);
    bytes.extend(stockage());
    assert_eq!(find_storage_interface(&bytes).map(|i| i.number), Some(3));
}

#[test]
fn refuse_uas_un_point_manquant_et_des_descripteurs_tronques() {
    let uas = [9, 4, 0, 0, 2, CLASS_STORAGE, SUBCLASS_SCSI, 0x62, 0];
    assert_eq!(find_storage_interface(&uas), None);
    let complet = stockage();
    assert_eq!(find_storage_interface(&complet[..complet.len() - 7]), None);
    assert_eq!(find_storage_interface(&complet[..complet.len() - 3]), None);
    assert_eq!(find_storage_interface(&[0, 0]), None);
    assert_eq!(find_storage_interface(&[]), None);
}

#[cfg(target_pointer_width = "64")]
#[test]
fn numeros_d_appel_du_noyau() {
    assert_eq!(USBDEVFS_CONTROL, 0xC018_5500);
    assert_eq!(USBDEVFS_BULK, 0xC018_5502);
    assert_eq!(USBDEVFS_CLAIMINTERFACE, 0x8004_550F);
    assert_eq!(USBDEVFS_RELEASEINTERFACE, 0x8004_5510);
    assert_eq!(USBDEVFS_IOCTL, 0xC010_5512);
    assert_eq!(USBDEVFS_CLEAR_HALT, 0x8004_5515);
    assert_eq!(USBDEVFS_DISCONNECT, 0x5516);
    assert_eq!(USBDEVFS_CONNECT, 0x5517);
}

#[test]
fn sans_memoire_le_transfert_recommence_plus_petit_et_le_reste() -> io::Result<()> {
    let mut limit = TRANSFER_START;
    let mut sizes = Vec::new();
    let done = with_shrinking_limit(&mut limit, 1 << 20, |size| {
        sizes.push(size);
        if size > 64 << 10 {
            Err(io::Error::from_raw_os_error(libc::ENOMEM))
        } else {
            Ok(size)
        }
    })?;
    assert_eq!(sizes, [256 << 10, 128 << 10, 64 << 10]);
    assert_eq!((done, limit), (64 << 10, 64 << 10));
    Ok(())
}

/// Out of memory in the middle of a receive: smaller transfers until the buffer is full, and the limit
/// stays smaller for the next one.
#[test]
fn une_reception_sans_memoire_se_poursuit_plus_petite() -> io::Result<()> {
    let mut limit = TRANSFER_START;
    let mut asked = Vec::new();
    let done = receive_chained(1 << 20, &mut limit, |_, size| {
        asked.push(size);
        if size > 128 << 10 {
            Err(io::Error::from_raw_os_error(libc::ENOMEM))
        } else {
            Ok(size)
        }
    })?;
    assert_eq!(done, 1 << 20);
    assert_eq!(limit, 128 << 10);
    assert_eq!(asked.iter().filter(|size| **size == 128 << 10).count(), 8);
    Ok(())
}

#[test]
fn un_paquet_court_termine_la_reception() -> io::Result<()> {
    let mut limit = TRANSFER_START;
    let mut transfers = 0;
    let done = receive_chained(1 << 20, &mut limit, |done, size| {
        transfers += 1;
        Ok(if done == 0 { size } else { 13 })
    })?;
    assert_eq!((done, transfers), (TRANSFER_START + 13, 2));
    Ok(())
}

#[test]
fn au_plus_petit_le_manque_de_memoire_remonte() {
    let mut limit = TRANSFER_MIN;
    let result = with_shrinking_limit(&mut limit, 1 << 20, |_| {
        Err(io::Error::from_raw_os_error(libc::ENOMEM))
    });
    assert!(result.is_err());
}

#[test]
fn une_autre_erreur_ne_reduit_rien() {
    let mut limit = TRANSFER_START;
    let result = with_shrinking_limit(&mut limit, 1 << 20, |_| {
        Err(io::Error::from_raw_os_error(libc::EPIPE))
    });
    assert!(result.is_err());
    assert_eq!(limit, TRANSFER_START);
}

#[test]
fn les_erreurs_du_systeme_gardent_leur_sens() {
    let erreur = |code| bulk_error(io::Error::from_raw_os_error(code));
    assert!(matches!(erreur(libc::EPIPE), BulkError::Stall));
    assert!(matches!(erreur(libc::ENODEV), BulkError::Disconnected));
    assert!(matches!(erreur(libc::EIO), BulkError::Other(_)));
}
