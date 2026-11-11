use std::collections::HashMap;

use ring::digest::{SHA256, digest};
use ring::signature::{Ed25519KeyPair, KeyPair};

use super::*;

const VAULT_DISK: u64 = 256 << 20;
const BASE: &str = "https://catalogue.test/";
const GRUB: &[u8] = b"MZ grub de test";

fn sha256(bytes: &[u8]) -> String {
    bonk_core::write::hex(digest(&SHA256, bytes).as_ref())
}

/// A catalog signed by a test key, offering `grub` when given, and the files it names.
fn served_catalog(grub: Option<&[u8]>) -> ServedCatalog {
    let pair = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).expect("clé de test");
    let public: [u8; 32] = pair.public_key().as_ref().try_into().expect("32 octets");
    let fake = "c".repeat(64);
    let entry = grub.map_or_else(String::new, |bytes| {
        format!(
            r#","grub":{{"version":"2.14","released":"2026-01-14","build":"6e330c761ee82877","efi":{{"x86_64":{{"path":"grub/2.14/BOOTX64.EFI","sha256":"{}","size_bytes":{}}}}},"source":{{"path":"grub/2.14/grub-2.14.tar.xz","sha256":"{fake}","size_bytes":1}},"source_signature":{{"path":"grub/2.14/grub-2.14.tar.xz.sig","sha256":"{fake}","size_bytes":1}}}}"#,
            sha256(bytes),
            bytes.len()
        )
    });
    let expires = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("horloge")
        .as_secs()
        + 3600;
    let manifest = format!(
        r#"{{"schema_version":1,"sequence":1,"expires_unix":{expires},"families":[],"images":[]{entry}}}"#
    )
    .into_bytes();
    let mut files = HashMap::new();
    files.insert(
        format!("{BASE}catalog.json.sig"),
        pair.sign(&manifest).as_ref().to_vec(),
    );
    files.insert(format!("{BASE}catalog.json"), manifest);
    if let Some(bytes) = grub {
        files.insert(format!("{BASE}grub/2.14/BOOTX64.EFI"), bytes.to_vec());
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
    assert_eq!(value["grub"]["sha256"], sha256(GRUB));
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

/// The second preparation needs no network: GRUB comes from the cache, checked again.
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

/// A command missing from this guard would let `--fixture` reach a real disk.
#[test]
fn le_coffre_refuse_fixture_sur_un_vrai_disque() {
    use clap::Parser;

    let conflict = |args: &[&str]| {
        let cli = crate::Cli::try_parse_from(args).expect("ligne valide");
        crate::fixture_conflict(&cli.command)
    };
    assert!(conflict(&["bonk", "vault", "list", "usb-a"]).is_some());
    assert!(conflict(&["bonk", "vault", "init", "usb-a", "--yes"]).is_some());
    assert!(conflict(&["bonk", "vault", "init", "usb-a", "--dry-run"]).is_none());
    assert!(conflict(&["bonk", "vault", "list", "a.img", "--simulate"]).is_none());
}

/// The window lists real disks and starts `bonk write` without the capture.
#[test]
fn la_fenetre_refuse_fixture() {
    use clap::Parser;

    let conflict = |args: &[&str]| {
        let cli = crate::Cli::try_parse_from(args).expect("ligne valide");
        crate::fixture_conflict(&cli.command)
    };
    assert!(conflict(&["bonk"]).is_some());
    assert!(conflict(&["bonk", "gui"]).is_some());
    assert!(conflict(&["bonk", "list"]).is_none());
}

/// An ISO the menu can start, as a file named `name` in the temporary directory.
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

/// Checked on the file alone: the drive is not even opened.
#[test]
fn une_iso_que_le_menu_ne_demarre_pas_est_refusee_avant_le_disque() {
    let (scenario, _cache) = vault_scenario(Some(GRUB), "cache-refus");
    let inconnue = iso_file("inconnue.iso", &[("boot/vmlinuz", 1)]);
    let espace = iso_file(
        "avec espace.iso",
        &[("live/vmlinuz", 1), ("live/initrd.img", 1)],
    );
    let extension = iso_file(
        "debian-live.img",
        &[("live/vmlinuz", 1), ("live/initrd.img", 1)],
    );
    let pas_iso = std::env::temp_dir().join(format!("bonk-job-{}-zero.iso", std::process::id()));
    std::fs::write(&pas_iso, vec![0u8; 64 * 2048]).expect("fichier de test");
    for path in [&inconnue, &espace, &extension, &pas_iso] {
        let arg = path.display().to_string();
        let (code, _, err) = scenario.run(&["vault", "add", "usb-a", &arg, "--yes"]);
        assert_eq!(code, EXIT_USAGE, "{err}");
        assert!(err.contains("E7004"), "{err}");
    }
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    for path in [&inconnue, &espace, &extension, &pas_iso] {
        let _ = std::fs::remove_file(path);
    }
}
