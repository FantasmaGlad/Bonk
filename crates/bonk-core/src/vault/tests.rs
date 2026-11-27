use crate::block::BlockDevice;
use crate::format::NewFile;
use crate::format::exfat::read::Reader;
use crate::format::table::{self, TableKind};
use crate::format::{Filesystem, FormatError, Ids, exfat, fat32};
use crate::partition::{ESP_GUID_TEXT, Severity, inspect};

use super::*;
use crate::block::MemoryDevice;
use crate::format::exfat::tree;
use crate::partition::read_range;

const MIB: u64 = 1 << 20;
const GIB: u64 = 1 << 30;

fn ids() -> Ids {
    Ids::from_seed(b"vault")
}

fn bios_grub() -> BiosGrub {
    let mut boot = vec![0xEB; 512];
    boot[510..].copy_from_slice(&[0x55, 0xAA]);
    let mut core = vec![0xC0; 3 * 512];
    core[400..512].fill(0);
    BiosGrub { boot, core }
}

fn boot_with(efi: &[u8]) -> Result<VaultBoot, FormatError> {
    Ok(VaultBoot {
        files: boot_files(&[("x86_64", efi.to_vec())], &default_families())?,
        bios: Some(bios_grub()),
    })
}

fn boot() -> VaultBoot {
    let boot = boot_with(&b"MZ grub x86_64".repeat(1000));
    assert!(boot.is_ok(), "{boot:?}");
    boot.unwrap_or_default()
}

fn init(dev: &mut MemoryDevice, table: TableKind) -> Result<VaultPlan, FormatError> {
    let plan = plan(dev.size(), dev.block_size(), table)?;
    write_table(dev, &plan, &ids())?;
    write_volumes(dev, &plan, &ids(), &boot())?;
    Ok(plan)
}

#[test]
fn une_iso_ubuntu_laisse_son_emplacement_et_le_retrait_l_efface() -> Result<(), FormatError> {
    let mut dev = MemoryDevice::new(256 << 20, 512)?;
    let plan = init(&mut dev, TableKind::Mbr)?;
    let data: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    let added = add_image(
        &mut dev,
        &plan,
        "ubuntu.iso",
        data.len() as u64,
        true,
        &mut data.as_slice(),
        &mut |_| true,
    )?;
    let names: Vec<String> = images(&dev, &plan)?.into_iter().map(|f| f.path).collect();
    assert_eq!(names, ["ubuntu.iso"]);
    let span = vault_span(&plan);
    let reader = Reader::open(&dev, span.first_lba, span.sectors)?;
    let all: Vec<String> = reader.files()?.into_iter().map(|f| f.path).collect();
    assert_eq!(all, ["ubuntu.iso", "ubuntu.iso.bonk"]);
    let start = plan.vault.first_lba * 512 + added.volume_offset;
    assert_eq!(read_range(&dev, start, data.len() as u64)?, data);
    assert_eq!(
        location_file(added.volume_offset),
        format!("set bonk_offset={}\n", added.volume_offset).into_bytes()
    );
    remove_image(&mut dev, &plan, "ubuntu.iso")?;
    let reader = Reader::open(&dev, span.first_lba, span.sectors)?;
    assert!(reader.files()?.is_empty());
    add_image(
        &mut dev,
        &plan,
        "arch.iso",
        3,
        false,
        &mut &b"iso"[..],
        &mut |_| true,
    )?;
    let reader = Reader::open(&dev, span.first_lba, span.sectors)?;
    assert_eq!(reader.files()?.len(), 1);
    Ok(())
}

#[test]
fn mettre_a_jour_le_demarrage_garde_les_iso() -> Result<(), FormatError> {
    let mut dev = MemoryDevice::new(256 << 20, 512)?;
    let plan = init(&mut dev, TableKind::Gpt)?;
    let data: Vec<u8> = (0..2_000_000u32).map(|i| (i % 253) as u8).collect();
    let added = add_image(
        &mut dev,
        &plan,
        "debian-live.iso",
        data.len() as u64,
        false,
        &mut data.as_slice(),
        &mut |_| true,
    )?;
    assert!(boot_matches(&dev, &plan, &boot())?);
    let newer = boot_with(&b"MZ grub x86_64, version suivante".repeat(2000))?;
    assert!(!boot_matches(&dev, &plan, &newer)?);
    let without_bios = VaultBoot {
        bios: None,
        ..boot()
    };
    assert!(boot_matches(&dev, &plan, &without_bios)?);

    let ids = Ids::from_seed(b"mise a jour");
    write_boot(&mut dev, &plan, &ids, &newer)?;
    verify_boot(&dev, &plan, &ids, &newer)?;
    assert!(boot_matches(&dev, &plan, &newer)?);
    assert_eq!(detect(&dev)?, Some(plan));
    let names: Vec<String> = images(&dev, &plan)?.into_iter().map(|f| f.path).collect();
    assert_eq!(names, ["debian-live.iso"]);
    verify_image(&dev, &plan, "debian-live.iso", &added.sha256, &mut |_| true)?;
    Ok(())
}

#[test]
fn familles_reconnues_comme_par_le_menu() -> std::io::Result<()> {
    use crate::iso9660::{Iso, testing::image};
    use std::io::Cursor;
    let families = default_families();
    let menu = menu(&families);
    let family = |label: &str, files: &[(&str, u32)]| {
        iso_kind(&mut Iso::open(Cursor::new(image(label, files)))?, &families)
            .map(|kind| kind.map(|kind| kind.family.id))
    };
    for expected in &families {
        for set in &expected.detect {
            for file in set {
                assert!(menu.contains(&format!("(probe)/{file}")), "{file}");
            }
            let present: Vec<(&str, u32)> = set.iter().map(|f| (f.as_str(), 1)).collect();
            let label = format!(
                "{}Test",
                expected.label_prefix.as_deref().unwrap_or("Live-")
            );
            assert_eq!(
                family(&label, &present)?,
                Some(expected.id.clone()),
                "{label}"
            );
            assert_eq!(family("X", &present[..1])?, None, "{}", expected.id);
        }
    }
    let kiwi = [
        ("boot/x86_64/loader/linux", 1),
        ("boot/x86_64/loader/initrd", 1),
    ];
    assert_eq!(family("openSUSE-Live", &kiwi)?, None);
    let both = [
        ("live/vmlinuz", 1),
        ("live/initrd.img", 1),
        ("casper/vmlinuz", 1),
        ("casper/initrd", 1),
    ];
    assert_eq!(family("X", &both)?, Some("debian_live".to_owned()));
    Ok(())
}

#[test]
fn processeur_reconnu_comme_par_le_menu() -> std::io::Result<()> {
    use crate::iso9660::{Iso, testing::image};
    use std::io::Cursor;
    let arch = |label: &str, files: &[(&str, u32)]| {
        iso_kind(
            &mut Iso::open(Cursor::new(image(label, files)))?,
            &default_families(),
        )
        .map(|kind| kind.map(|kind| kind.arch))
    };
    let menu = menu(&default_families());
    for (file, _) in ARCH_RULES {
        assert!(menu.contains(&format!("(probe)/{file}")), "{file}");
    }
    let casper = [("casper/vmlinuz", 1), ("casper/initrd", 1)];
    assert_eq!(arch("Ubuntu", &casper)?, Some(IsoArch::X86_64));
    let arm = [
        ("casper/vmlinuz", 1),
        ("casper/initrd", 1),
        ("EFI/boot/bootaa64.efi", 1),
    ];
    assert_eq!(arch("Ubuntu", &arm)?, Some(IsoArch::Aarch64));
    let anaconda = [
        ("images/pxeboot/vmlinuz", 1),
        ("images/pxeboot/initrd.img", 1),
        ("images/install.img", 1),
        ("EFI/BOOT/BOOTAA64.EFI", 1),
    ];
    assert_eq!(
        arch("AlmaLinux-10-3-aarch64-dvd", &anaconda)?,
        Some(IsoArch::Aarch64)
    );
    assert_eq!(
        arch("AlmaLinux-10-3-x86_64-dvd", &anaconda[..3])?,
        Some(IsoArch::X86_64)
    );
    // Fedora's ARM64 image also carries an x86_64 loader: its kernel folder decides.
    let fedora = [
        ("boot/aarch64/loader/linux", 1),
        ("boot/aarch64/loader/initrd", 1),
        ("EFI/BOOT/bootx64.efi", 1),
    ];
    assert_eq!(arch("Fedora-WS-Live-44", &fedora)?, Some(IsoArch::Aarch64));
    let fedora = [
        ("boot/x86_64/loader/linux", 1),
        ("boot/x86_64/loader/initrd", 1),
        ("EFI/BOOT/BOOTAA64.EFI", 1),
    ];
    assert_eq!(arch("Fedora-WS-Live-44", &fedora)?, Some(IsoArch::X86_64));
    Ok(())
}

#[test]
fn plan_mbr_exfat_d_abord_puis_esp_de_64_mio_en_fin() -> Result<(), FormatError> {
    let plan = plan(GIB, 512, TableKind::Mbr)?;
    assert_eq!(plan.vault.first_lba, 2048);
    assert_eq!(plan.efi.last_lba, GIB / 512 - 1);
    assert_eq!(plan.efi.sectors() * 512, 64 * MIB);
    assert_eq!(plan.vault.last_lba + 1, plan.efi.first_lba);
    assert_eq!(plan.efi.first_lba % 2048, 0);
    Ok(())
}

#[test]
fn plan_gpt_laisse_la_place_de_la_table_de_secours() -> Result<(), FormatError> {
    let plan = plan(GIB, 512, TableKind::Gpt)?;
    assert_eq!(plan.efi.last_lba, GIB / 512 - 34);
    assert!(plan.efi.sectors() * 512 >= 64 * MIB);
    Ok(())
}

#[test]
fn esp_agrandie_sur_secteurs_de_4_kio() -> Result<(), FormatError> {
    let plan = plan(2 * GIB, 4096, TableKind::Mbr)?;
    let bytes = plan.efi.sectors() * 4096;
    assert!(bytes > 256 * MIB && bytes < 300 * MIB, "{bytes}");
    assert!(fat32::geometry(plan.efi.sectors(), 4096).is_ok());
    Ok(())
}

#[test]
fn refuse_trop_petit_et_mbr_trop_grand() {
    assert!(matches!(
        plan(100 * MIB, 512, TableKind::Mbr),
        Err(FormatError::TooSmall(_))
    ));
    assert!(matches!(
        plan(4 << 40, 512, TableKind::Mbr),
        Err(FormatError::TooLarge(_))
    ));
    assert!(plan(4 << 40, 512, TableKind::Gpt).is_ok());
}

#[test]
fn ecrit_relu_et_reconnu_en_mbr_et_gpt() -> Result<(), Box<dyn std::error::Error>> {
    for table in TableKind::ALL {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        let plan = init(&mut dev, table)?;
        verify(&dev, &plan, &ids(), &boot())?;
        assert_eq!(detect(&dev)?, Some(plan));
        assert!(images(&dev, &plan)?.is_empty());
        let layout = inspect(&dev)?;
        assert!(
            layout
                .findings
                .iter()
                .all(|finding| finding.severity == Severity::Info),
            "{:?}",
            layout.findings
        );
    }
    Ok(())
}

#[test]
fn types_et_noms_des_partitions() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(256 * MIB, 512)?;
    init(&mut dev, TableKind::Gpt)?;
    let gpt = inspect(&dev)?.gpt.ok_or("pas de GPT")?;
    assert_eq!(gpt.entries[0].name, "BonkVault");
    assert_eq!(gpt.entries[1].name, "BonkEFI");
    assert_eq!(gpt.entries[1].type_guid, ESP_GUID_TEXT);
    let mut dev = MemoryDevice::new(256 * MIB, 512)?;
    init(&mut dev, TableKind::Mbr)?;
    let mbr = inspect(&dev)?.mbr.ok_or("pas de MBR")?;
    let kinds: Vec<u8> = mbr.entries.iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [0x07, 0xEF]);
    let active: Vec<bool> = mbr.entries.iter().map(|e| e.bootable).collect();
    assert_eq!(active, [true, false]);
    Ok(())
}

#[test]
fn une_cle_formatee_n_est_pas_un_coffre() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(256 * MIB, 512)?;
    let part = table::write_table(
        &mut dev,
        TableKind::Mbr,
        Filesystem::Exfat,
        Some(VAULT_LABEL),
        &ids(),
        false,
    )?;
    exfat::write(
        &mut dev,
        &exfat::Volume {
            first_lba: part.first_lba,
            sectors: part.sectors(),
            label: Some(VAULT_LABEL),
            volume_id: 1,
        },
    )?;
    assert_eq!(detect(&dev)?, None);
    assert_eq!(detect(&MemoryDevice::new(8 * MIB, 512)?)?, None);
    Ok(())
}

#[test]
fn liste_les_iso_sans_les_autres_fichiers() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(256 * MIB, 512)?;
    let plan = init(&mut dev, TableKind::Mbr)?;
    let file = |path: &str, size| NewFile {
        path: path.split('/').map(str::to_owned).collect(),
        size,
    };
    let volume = exfat_volume(&plan, &ids());
    let layout = tree::layout(
        &volume,
        512,
        &[
            file("debian-13.iso", 4 * MIB),
            file("Linux/Fedora.ISO", MIB),
            file("Linux/lisez-moi.txt", 10),
        ],
        &[],
    )?;
    layout.write_structures(&mut dev)?;
    layout.finish(&mut dev, &volume)?;
    assert_eq!(detect(&dev)?, Some(plan));
    let found: Vec<String> = images(&dev, &plan)?.into_iter().map(|f| f.path).collect();
    assert_eq!(found, ["Linux/Fedora.ISO", "debian-13.iso"]);
    Ok(())
}

#[test]
fn les_fichiers_de_demarrage_portent_les_noms_uefi_et_le_menu() -> Result<(), FormatError> {
    let files = boot_files(
        &[
            ("x86_64", vec![1]),
            ("aarch64", vec![2]),
            ("riscv64", vec![3]),
        ],
        &default_families(),
    )?;
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "EFI/BOOT/BOOTX64.EFI",
            "EFI/BOOT/BOOTAA64.EFI",
            "EFI/BOOT/BOOTRISCV64.EFI",
            "bonk/grub.cfg",
            "bonk/families.json",
            "bonk/casper/20iso_scan",
            "bonk/udev/99-bonk-vault-iso.rules"
        ]
    );
    assert!(VAULT_ISO_RULE.contains(&format!("ENV{{ID_FS_LABEL}}==\"{VAULT_LABEL}\"")));
    assert!(VAULT_ISO_RULE.contains("IMPORT{cmdline}=\"bonk.offset\""));
    let bound = VAULT_ISO_RULE.find("losetup");
    let probed = VAULT_ISO_RULE
        .find("udevadm trigger --action=change --subsystem-match=block --sysname-match=loop*");
    assert!(
        matches!((bound, probed), (Some(b), Some(p)) if b < p),
        "{VAULT_ISO_RULE}"
    );
    let menu = menu(&default_families());
    assert!(menu.contains("bonk.offset=$bonk_offset"));
    for (path, _) in INITRD_SCRIPTS {
        let beside_menu = path.trim_start_matches("bonk/");
        assert!(
            menu.contains(&format!(":$bonk_dir/{beside_menu}")),
            "{path}"
        );
    }
    assert!(files[3].data.starts_with(b"set timeout="));
    assert!(matches!(
        boot_files(&[("mips", vec![])], &default_families()),
        Err(FormatError::Unsupported(_))
    ));
    Ok(())
}

#[test]
fn un_octet_de_grub_change_fait_echouer_la_relecture() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(256 * MIB, 512)?;
    let plan = init(&mut dev, TableKind::Mbr)?;
    let mut other = boot();
    other.files[0].data[10] ^= 1;
    assert!(verify(&dev, &plan, &ids(), &other).is_err());
    let mut other = boot();
    if let Some(grub) = &mut other.bios {
        grub.core[600] ^= 1;
    }
    assert!(verify(&dev, &plan, &ids(), &other).is_err());
    verify(&dev, &plan, &ids(), &boot())?;
    Ok(())
}

#[test]
fn un_plan_pour_un_autre_disque_n_ecrit_rien() -> Result<(), Box<dyn std::error::Error>> {
    let mut dev = MemoryDevice::new(256 * MIB, 512)?;
    let other = plan(GIB, 512, TableKind::Mbr)?;
    assert!(matches!(
        write_table(&mut dev, &other, &ids()),
        Err(FormatError::Mismatch(_))
    ));
    assert!(dev.as_bytes().iter().all(|b| *b == 0));
    Ok(())
}

#[test]
fn les_familles_par_defaut_sont_valides_et_uniques() {
    let families = default_families();
    assert_eq!(families.len(), 11);
    for family in &families {
        assert_eq!(family_errors(family), Vec::<String>::new());
    }
    let mut ids: Vec<&str> = families.iter().map(|family| family.id.as_str()).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), families.len());
}

#[test]
fn une_famille_qui_pourrait_sortir_du_menu_est_refusee() {
    let good = default_families().remove(0);
    type Spoil = fn(&mut VaultFamily);
    let cases: [(&str, Spoil); 9] = [
        ("identifier", |f| f.id = "Debian".to_owned()),
        ("name", |f| f.name = "Debian\" ; reboot".to_owned()),
        ("detect", |f| {
            f.detect = vec![vec!["../live/vmlinuz".to_owned()]]
        }),
        ("detect", |f| f.detect = vec![]),
        ("label prefix", |f| {
            f.label_prefix = Some("Fedora.*".to_owned())
        }),
        ("kernel", |f| f.kernel = "/live/vmlinuz".to_owned()),
        ("options", |f| f.options = "boot=live $root".to_owned()),
        ("options", |f| f.options = "findiso={iso}".to_owned()),
        ("initrd extra", |f| f.initrd_extra = Some("a b".to_owned())),
    ];
    for (reason, spoil) in cases {
        let mut family = good.clone();
        spoil(&mut family);
        let errors = family_errors(&family);
        assert!(
            errors.iter().any(|error| error.contains(reason)),
            "{reason}: {errors:?}"
        );
    }
}

#[test]
fn une_famille_servie_en_donnees_entre_au_menu_sans_code() -> Result<(), Box<dyn std::error::Error>>
{
    let opensuse: VaultFamily = serde_json::from_str(
        r#"{"id":"opensuse_live","name":"openSUSE live","detect":[["boot/x86_64/loader/linux","boot/x86_64/loader/initrd"]],
            "label_prefix":"openSUSE","kernel":"boot/{cpu}/loader/linux","initrd":"boot/{cpu}/loader/initrd",
            "options":"iso-scan/filename={path} root=live:CDLABEL={label} rd.live.image"}"#,
    )?;
    assert_eq!(family_errors(&opensuse), Vec::<String>::new());
    let mut families = default_families();
    families.push(opensuse);
    let menu = menu(&families);
    assert!(menu.contains(
        "if [ -z \"$family\" -a -f (probe)/boot/x86_64/loader/linux -a -f (probe)/boot/x86_64/loader/initrd ]; then\n    if regexp '^openSUSE' \"$isolabel\"; then set family=opensuse_live; fi\n  fi"
    ));
    assert!(menu.contains("menuentry \"openSUSE live: $path\""));
    assert!(menu.contains(
        "linux (loop)/boot/$5/loader/linux iso-scan/filename=$2 root=live:CDLABEL=$4 rd.live.image"
    ));
    Ok(())
}

#[test]
fn le_coffre_garde_les_familles_de_son_menu() -> Result<(), Box<dyn std::error::Error>> {
    let mut families = default_families();
    families.retain(|family| family.id != "arch");
    let ids_of =
        |families: &[VaultFamily]| families.iter().map(|f| f.id.clone()).collect::<Vec<_>>();
    for (written, expected) in [
        (true, ids_of(&families)),
        (false, ids_of(&default_families())),
    ] {
        let mut dev = MemoryDevice::new(256 * MIB, 512)?;
        let plan = plan(dev.size(), dev.block_size(), TableKind::Gpt)?;
        write_table(&mut dev, &plan, &ids())?;
        let mut files = boot_files(&[("x86_64", b"MZ grub".repeat(100))], &families)?;
        if !written {
            files.retain(|file| file.path != FAMILIES_PATH);
        }
        let boot = VaultBoot { files, bios: None };
        write_volumes(&mut dev, &plan, &ids(), &boot)?;
        assert_eq!(ids_of(&vault_families(&dev, &plan)), expected, "{written}");
    }
    Ok(())
}

#[test]
fn une_famille_servie_illisible_ou_refusee_est_laissee_de_cote() {
    let mut debian = serde_json::to_value(&default_families()[0]).unwrap_or_default();
    let mut unsafe_family = debian.clone();
    unsafe_family["options"] = serde_json::json!("boot=live; reboot");
    debian["id"] = serde_json::json!("debian_bis");
    let served = [serde_json::json!({"id": 3}), unsafe_family, debian];
    let kept: Vec<String> = families_from(&served).into_iter().map(|f| f.id).collect();
    assert_eq!(kept, ["debian_bis"]);
    assert_eq!(families_from(&served[..2]), default_families());
}

#[test]
fn une_image_lue_par_positions_dit_sa_famille_ou_rien() -> std::io::Result<()> {
    use crate::iso9660::testing::image;
    use crate::read_at::ReadAt;
    struct Broken;
    impl ReadAt for Broken {
        fn read_exact_at(&self, _: u64, _: &mut [u8]) -> std::io::Result<()> {
            Err(std::io::Error::other("network"))
        }
    }
    let families = default_families();
    let live = image("X", &[("live/vmlinuz", 1), ("live/initrd.img", 1)]);
    assert_eq!(
        vault_match(&live[..], live.len() as u64, &families)?,
        Some(VaultMatch {
            family: "debian_live".to_owned(),
            name: "Debian live".to_owned(),
            arch: IsoArch::X86_64,
        })
    );
    let other = image("X", &[("boot/kernel", 1)]);
    assert_eq!(
        vault_match(&other[..], other.len() as u64, &families)?,
        None
    );
    for not_iso in [vec![0u8; 64 << 10], vec![0u8; 100]] {
        assert_eq!(
            vault_match(&not_iso[..], not_iso.len() as u64, &families)?,
            None
        );
    }
    assert!(vault_match(&Broken, 1 << 20, &families).is_err());
    Ok(())
}

#[test]
fn une_etiquette_qui_ne_convient_pas_laisse_essayer_la_famille_suivante() -> std::io::Result<()> {
    use crate::iso9660::{Iso, testing::image};
    use std::io::Cursor;
    let mut families = default_families();
    let mut installer = families
        .iter()
        .find(|family| family.id == "fedora_live")
        .cloned()
        .ok_or_else(|| std::io::Error::other("fedora_live"))?;
    installer.id = "kiwi_installer".to_owned();
    installer.label_prefix = Some("Kiwi-".to_owned());
    families.push(installer);
    let kiwi = [
        ("boot/x86_64/loader/linux", 1),
        ("boot/x86_64/loader/initrd", 1),
    ];
    let family = |label: &str| {
        iso_kind(&mut Iso::open(Cursor::new(image(label, &kiwi)))?, &families)
            .map(|kind| kind.map(|kind| kind.family.id))
    };
    assert_eq!(family("Fedora-WS-Live-44")?, Some("fedora_live".to_owned()));
    assert_eq!(family("Kiwi-Test")?, Some("kiwi_installer".to_owned()));
    assert_eq!(
        family("Install-Leap-16.0-x86_64")?,
        Some("agama".to_owned())
    );
    assert_eq!(family("openSUSE-Live")?, None);
    Ok(())
}
