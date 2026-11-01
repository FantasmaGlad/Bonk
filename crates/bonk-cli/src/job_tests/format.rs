//! Formatage : garde-fous, relecture, et passage de relais aux outils de l'hôte.

use super::*;

/// Taille de la partition d'un GPT sur le disque simulé.
fn new_partition_bytes() -> u64 {
    let plan = bonk_core::format::table::plan(
        DISK_SIZE as u64,
        512,
        bonk_core::format::table::TableKind::Gpt,
    )
    .expect("plan");
    plan.sectors() * 512
}

/// Le disque après formatage, avec le système de fichiers que le système y voit.
fn usb_with_new_partition(fs: &str) -> Disk {
    let mut disk = usb("S1");
    let mut part = partition("/dev/sdb1", &[], false);
    part.size_bytes = new_partition_bytes();
    part.fs_type = Some(fs.to_owned());
    disk.partitions = vec![part];
    disk
}

#[test]
fn formater_une_cle_passe_par_tous_les_garde_fous_puis_relit() {
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["format", "usb-a", "--label", "cle", "--yes", "--json"]);
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
    assert_eq!(value["filesystem"], "fat32");
    assert_eq!(value["table"], "gpt");
    assert_eq!(value["label"], "CLE");
    assert_eq!(value["verified"], true);
    assert_eq!(value["written_by"], "bonk");
    let written = scenario.written(1024);
    assert_eq!(&written[512..520], b"EFI PART");
}

#[test]
fn formater_sans_confirmation_ou_en_plan_ne_touche_a_rien() {
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["format", "usb-a"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");
    let (code, out, err) = scenario.run(&["format", "usb-a", "--dry-run"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Plan"), "{out}");
    assert!(out.contains("fat32 (default)"), "{out}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(1024).iter().all(|b| *b == 0));
}

#[test]
fn formater_un_disque_systeme_ou_interne_obeit_au_filtre_et_a_force() {
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["format", "nvme-sys", "--force", "--show-internal"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(scenario.log().is_empty());
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&["format", "sata-x", "--yes", "--show-internal"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains(CONFIRMATION_REQUIRED), "{err}");
    let (code, _, err) = scenario.run(&["format", "sata-x", "--force", "--show-internal"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn etiquette_invalide_refusee_avant_tout_geste() {
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "format",
        "usb-a",
        "--yes",
        "--label",
        "beaucoup trop longue",
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(scenario.log().is_empty());
}

fn exfat_capability(available: bool) -> Capability {
    if available {
        Capability::yes("format.exfat")
    } else {
        Capability::no(
            "format.exfat",
            Missing::MissingTool,
            "tool missing: mkfs.exfat",
        )
    }
}

#[test]
fn un_outil_absent_refuse_exfat_sans_toucher_au_disque() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(false)];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7002"), "{err}");
    assert!(err.contains("mkfs.exfat"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn exfat_ne_formate_jamais_l_ancienne_partition_encore_visible() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    // L'ancienne table reste visible un moment : même chemin /dev/sdb1, autre taille. C'est ce qu'a montré
    // l'essai sur la clé réelle.
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
        vec![usb_with_new_partition("exfat")],
    ];
    let (code, _, err) = scenario.run(&[
        "format", "usb-a", "--fs", "exfat", "--label", "Clé", "--yes",
    ]);
    assert_eq!(code, 0, "{err}");
    let log = scenario.log();
    assert!(log.contains(&"rescan:/dev/sdb".to_owned()), "{log:?}");
    assert_eq!(
        log.last().map(String::as_str),
        Some("format:/dev/sdb1:exfat:Clé")
    );
}

#[test]
fn exfat_sans_nouvelle_partition_ne_formate_rien() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7001"), "{err}");
    assert!(!scenario.log().iter().any(|l| l.starts_with("format:")));
}

#[test]
fn exfat_que_le_systeme_ne_reconnait_pas_est_un_echec() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
    ];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("does not recognize"), "{err}");
}

#[test]
fn capabilities_dit_le_pourquoi() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![Capability::yes("write"), exfat_capability(false)];
    let (code, out, _) = scenario.run(&["capabilities"]);
    assert_eq!(code, 0);
    assert!(out.contains("write"), "{out}");
    assert!(
        out.contains("format.exfat   no: tool missing (tool missing: mkfs.exfat)"),
        "{out}"
    );
    let (code, out, _) = scenario.run(&["capabilities", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["capabilities"][1]["missing"], "missing_tool");
}

/// FAT32 ne garde pas un fichier de 4 GiB : la sortie le dit et donne le remède ; les autres systèmes
/// n'ont pas cette limite à dire.
#[test]
fn fat32_dit_sa_limite_par_fichier_et_propose_exfat() {
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["format", "usb-a", "--fs", "fat32", "--dry-run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("4 GiB"), "{out}");
    assert!(out.contains("--fs exfat"), "{out}");

    let (code, json, err) =
        scenario.run(&["format", "usb-a", "--fs", "fat32", "--dry-run", "--json"]);
    assert_eq!((code, err.as_str()), (0, ""), "{json}");
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["file_size_limit_bytes"], (1u64 << 32) - 1);

    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    let (code, out, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--dry-run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(!out.contains("4 GiB"), "{out}");
    let (_, json, _) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--dry-run", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["file_size_limit_bytes"].is_null(), "{json}");
}

/// Annulé juste après l'ouverture : le disque reste intact, la réparation n'a pas commencé.
#[test]
fn reparer_s_annule_avant_d_ecrire() {
    let mut scenario = Scenario::new();
    scenario.cancel = true;
    let (code, _, err) = scenario.run(&["repair-gpt", "usb-a", "--yes"]);
    assert_eq!(code, crate::errors::EXIT_CANCELLED, "{err}");
    assert!(err.contains("E4003"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}
