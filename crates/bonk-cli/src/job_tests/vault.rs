use std::collections::HashMap;

use ring::digest::{SHA256, digest};
use ring::signature::{Ed25519KeyPair, KeyPair};

use super::*;

const VAULT_DISK: u64 = 256 << 20;
pub(crate) const BASE: &str = "https://catalogue.test/";
pub(crate) const GRUB: &[u8] = b"MZ grub de test";
const BIOS_BOOT_PATH: &str = "grub/2.14/bios/boot.img";
const BIOS_CORE_PATH: &str = "grub/2.14/bios/core.img";

fn sha256(bytes: &[u8]) -> String {
    bonk_core::write::hex(digest(&SHA256, bytes).as_ref())
}

pub(crate) fn served_catalog(grub: Option<&[u8]>) -> ServedCatalog {
    served_catalog_for(
        &grub
            .map(|bytes| vec![("x86_64", bytes)])
            .unwrap_or_default(),
    )
}

fn served_catalog_for(images: &[(&str, &[u8])]) -> ServedCatalog {
    served_catalog_with(images, None, "")
}

fn asset(path: &str, bytes: &[u8]) -> String {
    format!(
        r#"{{"path":"{path}","sha256":"{}","size_bytes":{}}}"#,
        sha256(bytes),
        bytes.len()
    )
}

fn served_catalog_with(
    images: &[(&str, &[u8])],
    bios: Option<(&[u8], &[u8])>,
    extra: &str,
) -> ServedCatalog {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("clé de test");
    let public: [u8; 32] = pair.public_key().as_ref().try_into().expect("32 octets");
    let fake = "c".repeat(64);
    let path = |arch: &str| format!("grub/2.14/{arch}.EFI");
    let efi: Vec<String> = images
        .iter()
        .map(|(arch, bytes)| format!(r#""{arch}":{}"#, asset(&path(arch), bytes)))
        .collect();
    let bios_entry = bios.map_or_else(String::new, |(boot, core)| {
        format!(
            r#","bios":{{"boot":{},"core":{}}}"#,
            asset(BIOS_BOOT_PATH, boot),
            asset(BIOS_CORE_PATH, core)
        )
    });
    let entry = if images.is_empty() {
        String::new()
    } else {
        format!(
            r#","grub":{{"version":"2.14","released":"2026-01-14","build":"6e330c761ee82877","efi":{{{}}}{bios_entry},"source":{{"path":"grub/2.14/grub-2.14.tar.xz","sha256":"{fake}","size_bytes":1}},"source_signature":{{"path":"grub/2.14/grub-2.14.tar.xz.sig","sha256":"{fake}","size_bytes":1}}}}"#,
            efi.join(",")
        )
    };
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("horloge")
        .as_secs()
        + 3600;
    let manifest = format!(
        r#"{{"schema_version":2,"sequence":1,"expires_unix":{expires},"families":[],"images":[]{entry}{extra}}}"#
    )
    .into_bytes();
    let mut files = HashMap::new();
    files.insert(
        format!("{BASE}catalog.json.sig"),
        pair.sign(&manifest).as_ref().to_vec(),
    );
    files.insert(format!("{BASE}catalog.json"), manifest);
    for (arch, bytes) in images {
        files.insert(format!("{BASE}{}", path(arch)), bytes.to_vec());
    }
    if let Some((boot, core)) = bios {
        files.insert(format!("{BASE}{BIOS_BOOT_PATH}"), boot.to_vec());
        files.insert(format!("{BASE}{BIOS_CORE_PATH}"), core.to_vec());
    }
    ServedCatalog {
        keys: vec![public],
        files: Rc::new(files),
    }
}

fn vault_scenario(grub: Option<&[u8]>, cache: &str) -> (Scenario, TempSource) {
    let mut scenario = Scenario::sized(VAULT_DISK);
    scenario.catalog = served_catalog(grub);
    (scenario, TempSource::dir(cache))
}

fn init_args<'a>(cache: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        "vault",
        "init",
        "usb-a",
        "--yes",
        "--json",
        "--catalog-url",
        BASE,
        "--cache-dir",
        cache,
    ];
    args.extend_from_slice(extra);
    args
}

#[test]
fn preparer_un_coffre_ecrit_grub_du_catalogue_puis_relit() {
    let (scenario, cache) = vault_scenario(Some(GRUB), "cache-preparer");
    let cache = cache.arg();
    let (code, out, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        scenario.log(),
        [
            "prepare:/dev/sdb",
            "unmount:usb-a",
            "open:/dev/sdb:ReadWrite",
            "drop_cache",
            "rescan:/dev/sdb"
        ]
    );
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["layout"]["table"], "mbr");
    assert_eq!(value["vault_label"], "BonkVault");
    assert_eq!(value["grub"]["version"], "2.14");
    assert_eq!(
        value["grub"]["efi"],
        serde_json::json!({"x86_64": sha256(GRUB)})
    );
    assert_eq!(value["verified"], true);

    let (code, out, err) = scenario.run(&["vault", "list", "usb-a", "--json"]);
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["images"], serde_json::json!([]));
    let listing = &scenario.log()[5..];
    assert!(
        listing.contains(&"open:/dev/sdb:ReadOnly".to_owned())
            && !listing.iter().any(|l| l.starts_with("unmount")),
        "{listing:?}"
    );
}

#[test]
fn grub_en_cache_sert_sans_reseau() {
    let (mut scenario, cache) = vault_scenario(Some(GRUB), "cache-hors-ligne");
    let cache = cache.arg();
    assert_eq!(scenario.run(&init_args(&cache, &[])).0, 0);
    scenario.catalog.files = Rc::new(HashMap::new());
    let (code, out, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"2.14\""), "{out}");
}

#[test]
fn sans_grub_au_catalogue_rien_n_est_ecrit() {
    let (scenario, cache) = vault_scenario(None, "cache-sans-grub");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3009") && err.contains("GRUB"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn grub_local_controle_contre_le_catalogue() {
    let (scenario, cache) = vault_scenario(Some(GRUB), "cache-local");
    let cache = cache.arg();
    let folder = TempSource::dir("grub-local");
    std::fs::create_dir_all(&folder.0).expect("dossier");
    std::fs::write(folder.0.join("BOOTX64.EFI"), b"autre grub").expect("fichier");
    let dir = folder.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &["--grub-dir", &dir]));
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("hash differs"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    std::fs::write(folder.0.join("BOOTX64.EFI"), GRUB).expect("fichier");
    let (code, _, err) = scenario.run(&init_args(&cache, &["--grub-dir", &dir]));
    assert_eq!(code, 0, "{err}");
}

#[test]
fn grub_de_chaque_processeur_ecrit_et_controle() {
    const ARM: &[u8] = b"MZ grub arm64 de test";
    let mut scenario = Scenario::sized(VAULT_DISK);
    scenario.catalog =
        served_catalog_for(&[("x86_64", GRUB), ("aarch64", ARM), ("mips64", b"MZ autre")]);
    let cache = TempSource::dir("cache-processeurs");
    let cache = cache.arg();
    let (code, out, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        value["grub"]["efi"],
        serde_json::json!({"aarch64": sha256(ARM), "x86_64": sha256(GRUB)})
    );
    assert_eq!(value["verified"], true);

    let folder = TempSource::dir("grub-processeurs");
    std::fs::create_dir_all(&folder.0).expect("dossier");
    std::fs::write(folder.0.join("BOOTX64.EFI"), GRUB).expect("fichier");
    let dir = folder.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &["--grub-dir", &dir]));
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("BOOTAA64.EFI"), "{err}");
    std::fs::write(folder.0.join("BOOTAA64.EFI"), ARM).expect("fichier");
    let (code, _, err) = scenario.run(&init_args(&cache, &["--grub-dir", &dir]));
    assert_eq!(code, 0, "{err}");
}

#[test]
fn sans_confirmation_ou_en_plan_rien_n_est_ecrit() {
    let scenario = Scenario::sized(VAULT_DISK);
    let (code, _, err) = scenario.run(&["vault", "init", "usb-a"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");
    let (code, out, err) = scenario.run(&["vault", "init", "usb-a", "--dry-run", "--table", "gpt"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Plan") && out.contains("Table: gpt"), "{out}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(1024).iter().all(|b| *b == 0));
}

#[test]
fn un_disque_trop_petit_ou_systeme_est_refuse_avant_tout_geste() {
    let mut scenario = Scenario::new();
    scenario.catalog = served_catalog(Some(GRUB));
    let cache = TempSource::dir("cache-petit");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7001"), "{err}");
    let (code, _, err) = scenario.run(&[
        "vault",
        "init",
        "nvme-sys",
        "--force",
        "--show-internal",
        "--dry-run",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn lister_un_disque_qui_n_est_pas_un_coffre_donne_e7003() {
    let scenario = Scenario::sized(VAULT_DISK);
    let (code, _, err) = scenario.run(&["vault", "list", "usb-a"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7003"), "{err}");
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 0, "{err}");
    let (code, _, err) = scenario.run(&["vault", "list", "usb-a"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7003"), "{err}");
}

#[test]
fn le_coffre_refuse_fixture_sur_un_vrai_disque() {
    use clap::Parser;

    let conflict = |args: &[&str]| {
        let cli = crate::Cli::try_parse_from(args).expect("ligne valide");
        crate::fixture_guard::fixture_conflict(&cli.command)
    };
    assert!(conflict(&["bonk", "vault", "list", "usb-a"]).is_some());
    assert!(conflict(&["bonk", "vault", "init", "usb-a", "--yes"]).is_some());
    assert!(conflict(&["bonk", "vault", "init", "usb-a", "--dry-run"]).is_none());
    assert!(conflict(&["bonk", "vault", "list", "a.img", "--simulate"]).is_none());
}

#[test]
fn la_fenetre_refuse_fixture() {
    use clap::Parser;

    let conflict = |args: &[&str]| {
        let cli = crate::Cli::try_parse_from(args).expect("ligne valide");
        crate::fixture_guard::fixture_conflict(&cli.command)
    };
    assert!(conflict(&["bonk"]).is_some());
    assert!(conflict(&["bonk", "gui"]).is_some());
    assert!(conflict(&["bonk", "list"]).is_none());
}

fn iso_file(name: &str, files: &[(&str, u32)]) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("bonk-job-{}-{name}", std::process::id()));
    let image = bonk_core::iso9660::testing::image("ARCH_202611", files);
    std::fs::write(&path, image).expect("ISO de test");
    path
}

#[test]
fn ajouter_puis_retirer_une_iso_sur_le_coffre() {
    let (scenario, cache) = vault_scenario(Some(GRUB), "cache-ajout");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let arch = iso_file(
        "archlinux-x86_64.iso",
        &[
            ("arch/boot/x86_64/vmlinuz-linux", 1),
            ("arch/boot/x86_64/initramfs-linux.img", 1),
        ],
    );
    let arch_arg = arch.display().to_string();
    let name = arch
        .file_name()
        .and_then(|n| n.to_str())
        .expect("nom")
        .to_owned();

    let (code, _, err) = scenario.run(&["vault", "add", "usb-a", &arch_arg]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");

    let (code, out, err) = scenario.run(&["vault", "add", "usb-a", &arch_arg, "--yes", "--json"]);
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["family"], "arch");
    assert_eq!(value["name"], name.as_str());
    assert_eq!(value["verified"], true);
    let (code, out, err) = scenario.run(&["vault", "list", "usb-a", "--json"]);
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["images"][0]["path"], name.as_str());

    let (code, _, err) = scenario.run(&["vault", "add", "usb-a", &arch_arg, "--yes"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E7006"), "{err}");

    let (code, out, err) = scenario.run(&["vault", "remove", "usb-a", &name, "--yes"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Removed from the vault"), "{out}");
    let (code, out, _) = scenario.run(&["vault", "list", "usb-a", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["images"], serde_json::json!([]));
    let (code, _, err) = scenario.run(&["vault", "remove", "usb-a", &name, "--yes"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E7006"), "{err}");
    let _ = std::fs::remove_file(&arch);
}

#[test]
fn un_installateur_anaconda_entre_au_coffre_avec_son_emplacement() {
    let (scenario, cache) = vault_scenario(Some(GRUB), "cache-anaconda");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let alma = iso_file(
        "AlmaLinux-10.3-aarch64-boot.iso",
        &[
            ("images/pxeboot/vmlinuz", 1),
            ("images/pxeboot/initrd.img", 1),
            ("images/install.img", 1),
            ("EFI/BOOT/BOOTAA64.EFI", 1),
        ],
    );
    let alma_arg = alma.display().to_string();
    let (code, out, err) = scenario.run(&["vault", "add", "usb-a", &alma_arg, "--yes", "--json"]);
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        (value["family"].as_str(), value["arch"].as_str()),
        (Some("anaconda"), Some("aarch64"))
    );
    let _ = std::fs::remove_file(&alma);
}

#[test]
fn une_image_du_catalogue_ne_se_telecharge_pas_sans_confirmation() {
    let (scenario, _cache) = vault_scenario(Some(GRUB), "cache-sans-oui");
    let (code, _, err) = scenario.run(&[
        "vault",
        "add",
        "usb-a",
        "--image",
        "almalinux-10-boot-aarch64",
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");
    let (code, _, err) = scenario.run(&[
        "vault",
        "add",
        "usb-a",
        "/x.iso",
        "--image",
        "almalinux-10-boot-aarch64",
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
}

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
fn une_iso_dont_le_nom_a_des_espaces_entre_avec_des_soulignes() {
    let (scenario, cache) = vault_scenario(Some(GRUB), "cache-espace");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let debian = iso_file(
        "debian live.iso",
        &[("live/vmlinuz", 1), ("live/initrd.img", 1)],
    );
    let arg = debian.display().to_string();
    let (code, out, err) = scenario.run(&["vault", "add", "usb-a", &arg, "--yes", "--json"]);
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let kept = value["name"].as_str().unwrap_or_default();
    assert!(
        kept.ends_with("debian_live.iso") && !kept.contains(' '),
        "{kept}"
    );
    let _ = std::fs::remove_file(&debian);
}

fn update_json(scenario: &Scenario, cache: &str, extra: &[&str]) -> serde_json::Value {
    let mut args = vec![
        "vault",
        "update",
        "usb-a",
        "--json",
        "--catalog-url",
        BASE,
        "--cache-dir",
        cache,
    ];
    args.extend_from_slice(extra);
    let (code, out, err) = scenario.run(&args);
    assert_eq!(code, 0, "{err}");
    serde_json::from_str(&out).expect("JSON")
}

#[test]
fn mettre_a_jour_grub_garde_les_iso() {
    let (mut scenario, cache) = vault_scenario(Some(GRUB), "cache-maj");
    let cache = cache.arg();
    let (code, _, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let arch = iso_file(
        "arch-maj.iso",
        &[
            ("arch/boot/x86_64/vmlinuz-linux", 1),
            ("arch/boot/x86_64/initramfs-linux.img", 1),
        ],
    );
    let arch_arg = arch.display().to_string();
    let (code, _, err) = scenario.run(&["vault", "add", "usb-a", &arch_arg, "--yes"]);
    assert_eq!(code, 0, "{err}");
    let run = |scenario: &Scenario, extra: &[&str]| update_json(scenario, &cache, extra);
    assert_eq!(run(&scenario, &[])["up_to_date"], true);

    scenario.catalog = served_catalog(Some(b"MZ grub de test, version suivante"));
    let check = run(&scenario, &[]);
    assert_eq!(
        (check["up_to_date"].clone(), check["updated"].clone()),
        (false.into(), false.into())
    );
    let done = run(&scenario, &["--yes"]);
    assert_eq!(
        (done["updated"].clone(), done["verified"].clone()),
        (true.into(), true.into())
    );
    assert_eq!(run(&scenario, &["--yes"])["up_to_date"], true);

    let (code, out, _) = scenario.run(&["vault", "list", "usb-a", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    let name = arch.file_name().and_then(|n| n.to_str()).expect("nom");
    assert_eq!(value["images"][0]["path"], name);
    let _ = std::fs::remove_file(&arch);
}

fn bios_images() -> (Vec<u8>, Vec<u8>) {
    let mut boot = vec![0xEB; 512];
    boot[510..].copy_from_slice(&[0x55, 0xAA]);
    let mut core = vec![0xC0; 1024];
    core[400..512].fill(0);
    (boot, core)
}

#[test]
fn le_coffre_demarre_aussi_en_bios_quand_le_catalogue_offre_grub_pour_le_bios() {
    let (boot, core) = bios_images();
    let (mut scenario, cache) = vault_scenario(Some(GRUB), "cache-bios");
    scenario.catalog = served_catalog_with(&[("x86_64", GRUB)], Some((&boot, &core)), "");
    let cache = cache.arg();
    let (code, out, err) = scenario.run(&init_args(&cache, &[]));
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["bios"], true);
    assert_eq!(
        value["grub"]["bios"],
        serde_json::json!({"boot": sha256(&boot), "core": sha256(&core)})
    );
    let mbr = scenario.written(512);
    assert_eq!((mbr[..3].to_vec(), mbr[0x1BE]), (vec![0xEB; 3], 0x80));
    assert_eq!(update_json(&scenario, &cache, &[])["up_to_date"], true);

    let mut older = core.clone();
    older[700] ^= 1;
    scenario.catalog = served_catalog_with(&[("x86_64", GRUB)], Some((&boot, &older)), "");
    let done = update_json(&scenario, &cache, &["--yes"]);
    assert_eq!(
        (done["updated"].clone(), done["bios"].clone()),
        (true.into(), true.into())
    );

    let folder = TempSource::dir("grub-local-sans-bios");
    std::fs::create_dir_all(&folder.0).expect("dossier");
    std::fs::write(folder.0.join("BOOTX64.EFI"), GRUB).expect("fichier");
    let dir = folder.arg();
    let (code, out, err) = scenario.run(&init_args(&cache, &["--grub-dir", &dir]));
    assert_eq!(code, 0, "{err}");
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(
        (value["bios"].clone(), value["grub"]["bios"].clone()),
        (false.into(), serde_json::json!({}))
    );
}
