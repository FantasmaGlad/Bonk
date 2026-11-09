use super::*;

const VAULT_DISK: u64 = 256 << 20;

#[test]
fn preparer_un_coffre_passe_par_les_garde_fous_puis_le_relit() {
    let scenario = Scenario::sized(VAULT_DISK);
    let (code, out, err) = scenario.run(&["vault", "init", "usb-a", "--yes", "--json"]);
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
    assert_eq!(value["layout"]["vault"]["first_lba"], 2048);
    assert_eq!(value["vault_label"], "BonkVault");
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
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["vault", "init", "usb-a", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7001"), "{err}");
    let (code, _, err) = scenario.run(&["vault", "init", "nvme-sys", "--force", "--show-internal"]);
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
