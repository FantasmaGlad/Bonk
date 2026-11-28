use std::collections::BTreeMap;

use bonk_core::vault::default_families;

use super::*;

fn asset(name: &str, data: &[u8]) -> Asset {
    Asset {
        path: format!("grub/{name}"),
        sha256: format!("sha256-{name}"),
        size_bytes: data.len() as u64,
    }
}

const X86: &[u8] = b"MZ grub x86_64";
const BOOT: &[u8] = b"boot.img";
const CORE: &[u8] = b"core.img";

fn grub(efi: &[(&str, &[u8])]) -> Grub {
    Grub {
        version: "2.14".to_owned(),
        released: "2026-01-01".to_owned(),
        build: "1".to_owned(),
        efi: efi
            .iter()
            .map(|(arch, data)| ((*arch).to_owned(), asset(arch, data)))
            .collect(),
        bios: Some(GrubBios {
            boot: asset("boot.img", BOOT),
            core: asset("core.img", CORE),
        }),
        source: asset("grub.tar.xz", b"source"),
        source_signature: asset("grub.tar.xz.sig", b"signature"),
    }
}

fn lecture(file: GrubFile<'_>, _: &Asset) -> Result<Vec<u8>, String> {
    Ok(match file {
        GrubFile::Efi(_) => X86.to_vec(),
        GrubFile::BiosBoot => BOOT.to_vec(),
        GrubFile::BiosCore => CORE.to_vec(),
    })
}

#[test]
fn un_seul_assemblage_donne_les_fichiers_et_leur_releve() -> Result<(), String> {
    let entry = grub(&[("x86_64", X86), ("riscv32", b"inconnu")]);
    let (boot, info) = entry.vault_boot(entry.bios.as_ref(), &default_families(), &lecture)?;
    assert!(
        boot.files
            .iter()
            .any(|f| f.path == "EFI/BOOT/BOOTX64.EFI" && f.data == X86)
    );
    assert_eq!(
        boot.bios.map(|b| (b.boot, b.core)),
        Some((BOOT.to_vec(), CORE.to_vec()))
    );
    assert_eq!(
        info.efi,
        BTreeMap::from([("x86_64".to_owned(), "sha256-x86_64".to_owned())])
    );
    assert_eq!(info.bios.keys().collect::<Vec<_>>(), ["boot", "core"]);
    assert_eq!((info.version.as_str(), info.build.as_str()), ("2.14", "1"));
    Ok(())
}

#[test]
fn sans_bios_retenu_le_coffre_ne_demarre_qu_en_uefi() -> Result<(), String> {
    let entry = grub(&[("x86_64", X86)]);
    let (boot, info) = entry.vault_boot(None, &default_families(), &lecture)?;
    assert!(boot.bios.is_none());
    assert_eq!(info.bios, BTreeMap::new());
    Ok(())
}

#[test]
fn aucune_image_ecrivable_et_lecture_refusee_sont_des_erreurs() {
    let inconnu = grub(&[("riscv32", b"inconnu")]);
    let erreur = inconnu
        .vault_boot(None, &default_families(), &lecture)
        .err();
    assert!(erreur.is_some_and(|e| e.contains("no GRUB image")));
    let entry = grub(&[("x86_64", X86)]);
    let refus = |_: GrubFile<'_>, _: &Asset| Err("empreinte fausse".to_owned());
    let erreur = entry
        .vault_boot(entry.bios.as_ref(), &default_families(), &refus)
        .err();
    assert_eq!(erreur.as_deref(), Some("empreinte fausse"));
}
