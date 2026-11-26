use super::vault::*;
use super::*;

#[test]
fn une_iso_que_le_menu_ne_demarre_pas_est_refusee_avant_le_disque() {
    let (scenario, _cache) = vault_scenario(Some(GRUB), "cache-refus");
    let inconnue = iso_file("inconnue.iso", &[("boot/vmlinuz", 1)]);
    let extension = iso_file(
        "debian-live.img",
        &[("live/vmlinuz", 1), ("live/initrd.img", 1)],
    );
    let pas_iso = std::env::temp_dir().join(format!("bonk-job-{}-zero.iso", std::process::id()));
    std::fs::write(&pas_iso, vec![0u8; 64 * 2048]).expect("fichier de test");
    for path in [&inconnue, &extension, &pas_iso] {
        let arg = path.display().to_string();
        let (code, _, err) = scenario.run(&["vault", "add", "usb-a", &arg, "--yes"]);
        assert_eq!(code, EXIT_USAGE, "{err}");
        assert!(err.contains("E7004"), "{err}");
    }
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    for path in [&inconnue, &extension, &pas_iso] {
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn le_menu_du_coffre_suit_les_familles_du_catalogue() {
    let mut families = bonk_core::vault::default_families();
    families.retain(|family| family.id != "arch");
    let served = format!(
        r#","vault_families":{}"#,
        serde_json::to_string(&families).expect("JSON")
    );
    let mut scenario = Scenario::sized(VAULT_DISK);
    scenario.catalog = served_catalog_with(&[("x86_64", GRUB)], None, &served);
    let cache = TempSource::dir("cache-familles");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let disk = scenario.written(VAULT_DISK as usize);
    let menu = bonk_core::vault::menu(&families);
    assert!(
        disk.windows(menu.len()).any(|w| w == menu.as_bytes()),
        "menu des familles servies"
    );
    let arch = iso_file(
        "arch-sans-famille.iso",
        &[
            ("arch/boot/x86_64/vmlinuz-linux", 1),
            ("arch/boot/x86_64/initramfs-linux.img", 1),
        ],
    );
    let arg = arch.display().to_string();
    let (code, _, err) = scenario.run(&["vault", "add", "usb-a", &arg, "--yes"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(
        err.contains("does not start Arch Linux yet") && err.contains("E7004"),
        "{err}"
    );
    let _ = std::fs::remove_file(&arch);
}

#[test]
fn ajouter_reconnait_une_famille_que_seul_le_catalogue_enregistre_connait() {
    let mut families = bonk_core::vault::default_families();
    families.push(bonk_core::vault::VaultFamily {
        id: "mint".to_owned(),
        name: "Linux Mint".to_owned(),
        detect: vec![vec![
            "casper/vmlinuz".to_owned(),
            "casper/initrd.lz".to_owned(),
        ]],
        label_prefix: None,
        kernel: "casper/vmlinuz".to_owned(),
        initrd: "casper/initrd.lz".to_owned(),
        options: "boot=casper iso-scan/filename={path}".to_owned(),
        location: false,
        initrd_extra: None,
    });
    let served = format!(
        r#","vault_families":{}"#,
        serde_json::to_string(&families).expect("JSON")
    );
    let mut scenario = Scenario::sized(VAULT_DISK);
    scenario.catalog = served_catalog_with(&[("x86_64", GRUB)], None, &served);
    let cache = TempSource::dir("cache-famille-servie");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let mint = iso_file(
        "linuxmint.iso",
        &[("casper/vmlinuz", 1), ("casper/initrd.lz", 1)],
    );
    let arg = mint.display().to_string();
    let (code, out, err) = scenario.run(&[
        "vault",
        "add",
        "usb-a",
        &arg,
        "--yes",
        "--json",
        "--cache-dir",
        &cache,
    ]);
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["family"], "mint");
    let _ = std::fs::remove_file(&mint);
}
