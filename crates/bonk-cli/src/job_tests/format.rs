use super::*;

fn new_partition_bytes() -> u64 {
    let plan = bonk_core::format::table::plan(
        DISK_SIZE as u64,
        512,
        bonk_core::format::table::TableKind::Gpt,
    )
    .expect("plan");
    plan.sectors() * 512
}

/// The disk after formatting, with the filesystem the system sees.
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

/// exFAT is written by Bonk like FAT32: no host tool, the same read-back.
#[test]
fn exfat_est_ecrit_par_bonk_sans_outil_de_l_hote() {
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "format", "usb-a", "--fs", "exfat", "--label", "Clé", "--yes", "--json",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(
        !scenario.log().iter().any(|l| l.starts_with("format:")),
        "{:?}",
        scenario.log()
    );
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["filesystem"], "exfat");
    assert_eq!(value["written_by"], "bonk");
    assert_eq!(value["verified"], true);
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

fn ntfs_capability(available: bool) -> Capability {
    if available {
        Capability::yes("format.ntfs")
    } else {
        Capability::no(
            "format.ntfs",
            Missing::MissingTool,
            "tool missing: mkfs.ntfs",
        )
    }
}

#[test]
fn un_outil_absent_refuse_ntfs_sans_toucher_au_disque() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![ntfs_capability(false)];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "ntfs", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7002"), "{err}");
    assert!(err.contains("mkfs.ntfs"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn ntfs_ne_formate_jamais_l_ancienne_partition_encore_visible() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![ntfs_capability(true)];
    // The old table stays visible for a while: same path /dev/sdb1, another size. Seen on the real key.
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
        vec![usb_with_new_partition("ntfs")],
    ];
    let (code, _, err) =
        scenario.run(&["format", "usb-a", "--fs", "ntfs", "--label", "Clé", "--yes"]);
    assert_eq!(code, 0, "{err}");
    let log = scenario.log();
    assert!(log.contains(&"rescan:/dev/sdb".to_owned()), "{log:?}");
    assert_eq!(
        log.last().map(String::as_str),
        Some("format:/dev/sdb1:ntfs:Clé")
    );
}

/// Between the new table and the host format, another key took the place: the host tool is never run on it.
#[test]
fn ntfs_ne_formate_pas_une_autre_cle_arrivee_entre_temps() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![ntfs_capability(true)];
    let mut autre = usb_with_new_partition("");
    autre.serial = Some("S2".to_owned());
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
        vec![autre],
    ];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "ntfs", "--yes"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(err.trim_end().ends_with("[E2002]"), "{err}");
    assert!(
        !scenario.log().iter().any(|l| l.starts_with("format:")),
        "{:?}",
        scenario.log()
    );
}

#[test]
fn ntfs_sans_nouvelle_partition_ne_formate_rien() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![ntfs_capability(true)];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "ntfs", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E7001"), "{err}");
    assert!(!scenario.log().iter().any(|l| l.starts_with("format:")));
}

#[test]
fn ntfs_que_le_systeme_ne_reconnait_pas_est_un_echec() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![ntfs_capability(true)];
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
    ];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "ntfs", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("does not recognize"), "{err}");
}

#[test]
fn capabilities_dit_le_pourquoi() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![Capability::yes("write"), ntfs_capability(false)];
    let (code, out, _) = scenario.run(&["capabilities"]);
    assert_eq!(code, 0);
    assert!(out.contains("write"), "{out}");
    assert!(
        out.contains("format.ntfs    no: tool missing (tool missing: mkfs.ntfs)"),
        "{out}"
    );
    let (code, out, _) = scenario.run(&["capabilities", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["capabilities"][1]["missing"], "missing_tool");
}

/// FAT32 cannot keep a 4 GiB file: the output says so with the remedy; other filesystems have no such limit
/// to report.
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

    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--dry-run"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(!out.contains("4 GiB"), "{out}");
    let (_, json, _) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--dry-run", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(value["file_size_limit_bytes"].is_null(), "{json}");
}

/// An 8 MiB GPT image at the start of the key: its backup GPT is missing at the end.
fn cle_a_reparer() -> Scenario {
    use bonk_core::format::table::{TableKind, write_table};
    use bonk_core::format::{Filesystem, Ids};
    let mut image = MemoryDevice::new(8 << 20, 512).expect("géométrie valide");
    write_table(
        &mut image,
        TableKind::Gpt,
        Filesystem::Fat32,
        None,
        &Ids::from_seed(b"cle"),
        false,
    )
    .expect("table");
    let scenario = Scenario::new();
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(0, image.as_bytes())
        .expect("copie");
    scenario
}

/// Once repaired, the table is read back on the key and by the system, as after a write.
#[test]
fn reparer_relit_la_table_sur_la_cle_et_par_le_systeme() {
    let scenario = cle_a_reparer();
    let (code, out, err) = scenario.run(&["repair-gpt", "usb-a", "--yes", "--json"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["needs_repair"], true, "{out}");
    assert!(
        value["findings_after"]
            .as_array()
            .is_some_and(|f| f.iter().all(|c| c != "gpt_backup_missing")),
        "{out}"
    );
    assert!(
        scenario
            .log()
            .iter()
            .any(|call| call.starts_with("rescan:")),
        "{:?}",
        scenario.log()
    );
}

/// A key that does not keep the end: the repair looked done, the read-back says otherwise.
#[test]
fn une_reparation_que_la_cle_ne_garde_pas_echoue() {
    let mut scenario = cle_a_reparer();
    scenario.faults.lost_after = Some(32 << 20);
    let (code, _, err) = scenario.run(&["repair-gpt", "usb-a", "--yes"]);
    assert_eq!(code, crate::errors::EXIT_RUNTIME_ERROR, "{err}");
    assert!(err.trim_end().ends_with("[E4005]"), "{err}");
}

/// Cancelled right after opening: the disk stays intact, the repair did not start.
#[test]
fn reparer_s_annule_avant_d_ecrire() {
    let mut scenario = Scenario::new();
    scenario.cancel = true;
    let (code, _, err) = scenario.run(&["repair-gpt", "usb-a", "--yes"]);
    assert_eq!(code, crate::errors::EXIT_CANCELLED, "{err}");
    assert!(err.contains("E4003"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}
