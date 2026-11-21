use bonk_core::BlockError;

use super::*;

#[test]
fn le_bios_n_est_annonce_qu_avec_des_secteurs_de_512_octets() {
    let image = WindowsImage {
        items: Vec::new(),
        directories: Vec::new(),
        signer: Signer::Unsigned,
        architectures: vec![Machine::X86_64],
        wim_parts: 0,
        bios_loader: true,
    };
    assert_eq!(
        image.describe(TableKind::Mbr, 512).firmware,
        [Firmware::Uefi, Firmware::Bios]
    );
    assert_eq!(
        image.describe(TableKind::Mbr, 4096).firmware,
        [Firmware::Uefi]
    );
    assert_eq!(
        image.describe(TableKind::Gpt, 512).firmware,
        [Firmware::Uefi]
    );
}

#[test]
fn un_mbr_au_dela_de_2_tio_est_refuse_par_le_code_du_formatage() {
    let error = partition_plan(4 << 40, 512, TableKind::Mbr).err();
    assert_eq!(error.map(|e| e.code), Some(Code::FormatFailed));
    assert!(partition_plan(4 << 40, 512, TableKind::Gpt).is_ok());
}

#[test]
fn un_support_disparu_pendant_la_copie_est_dit() {
    let full = io::Error::from(io::ErrorKind::StorageFull);
    let error = format_failure(Code::WriteFailed, FormatError::Device(BlockError::Io(full)));
    assert_eq!(error.code, Code::WriteFailed);
    assert!(error.message.contains("unplugged"), "{}", error.message);
}
