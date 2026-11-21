#![allow(clippy::expect_used)]

use super::*;
use crate::analysis::HEAD_LEN;
use crate::fat::Kind;
use crate::fat::testing::small;
use crate::hybrid::testing::{dvd_iso, dvd_iso_with_fat};
use crate::partition::testing::{ESP, gpt_image};
use crate::pe::testing::{ARM64, X86_64, name, pe};

const ESP_START: u64 = 2048;

fn head(image: &[u8]) -> &[u8] {
    &image[..image.len().min(HEAD_LEN)]
}

fn disk(fat: &[u8]) -> Vec<u8> {
    let sectors = ESP_START + fat.len() as u64 / 512 + 64;
    let end = ESP_START + fat.len() as u64 / 512 - 1;
    let mut image = gpt_image(sectors, &[(ESP_START, end, ESP, "EFI")]);
    let at = ESP_START as usize * 512;
    image[at..at + fat.len()].copy_from_slice(fat);
    image
}

#[test]
fn un_disque_gpt_dit_le_chargeur_de_sa_partition_efi() {
    let loader = pe(X86_64, &[]);
    let image = disk(&small(Kind::Fat12, &[("EFI/BOOT/BOOTX64.EFI", &loader)]).image);
    assert_eq!(
        loaders(image.as_slice(), head(&image)),
        [Loader {
            machine: Machine::X86_64,
            signer: Signer::Unsigned
        }]
    );
}

#[test]
fn une_iso_dit_chaque_chargeur_de_son_image_el_torito() {
    let shim = pe(X86_64, &name("Shim Signer"));
    let arm = pe(ARM64, &[]);
    let fat = small(
        Kind::Fat12,
        &[
            ("EFI/BOOT/BOOTX64.EFI", &shim),
            ("EFI/BOOT/BOOTAA64.EFI", &arm),
        ],
    );
    let image = dvd_iso_with_fat(&fat.image);
    assert_eq!(
        loaders(image.as_slice(), head(&image)),
        [
            Loader {
                machine: Machine::X86_64,
                signer: Signer::Other("Shim Signer".to_owned())
            },
            Loader {
                machine: Machine::Arm64,
                signer: Signer::Unsigned
            },
        ]
    );
}

#[test]
fn un_chargeur_d_un_autre_processeur_que_son_nom_est_ignore() {
    let wrong = pe(X86_64, &[]);
    let image = disk(&small(Kind::Fat16, &[("EFI/BOOT/BOOTAA64.EFI", &wrong)]).image);
    assert_eq!(loaders(image.as_slice(), head(&image)), []);
}

#[test]
fn sans_volume_efi_lisible_aucun_chargeur() {
    let unreadable = disk(&[0u8; 512 * 64]);
    let plain = vec![0u8; 1 << 20];
    for image in [unreadable, plain, dvd_iso(4)] {
        assert_eq!(loaders(image.as_slice(), head(&image)), []);
    }
}
