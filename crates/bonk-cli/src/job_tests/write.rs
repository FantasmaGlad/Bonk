use super::*;

#[test]
fn ecriture_complete_dans_l_ordre_des_garde_fous() {
    let source = TempSource::new("ordre", 1 << 20);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    // Unmount first, then open for writing, never the reverse; the table is reread only once the disk is
    // closed, otherwise the kernel keeps the old partition sizes.
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
    assert_eq!(scenario.written(1 << 20), source.bytes());
    assert!(out.contains("Unmounted: /run/media/u/A"), "{out}");
    assert!(out.contains("Verification: passed"), "{out}");
}

#[test]
fn une_table_non_relue_n_enleve_rien_a_l_ecriture() {
    let source = TempSource::new("table-non-relue", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.rescan_fails = true;
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        scenario.log().contains(&"rescan:/dev/sdb".to_owned()),
        "{:?}",
        scenario.log()
    );
    assert_eq!(scenario.written(1 << 20), source.bytes());
}

#[test]
fn une_ecriture_simulee_ne_fait_relire_aucune_table() {
    let source = TempSource::new("simulee-source", 4096);
    let cible = TempSource::from_bytes("simulee-cible", &[0; 4096]);
    let scenario = Scenario::new();
    let args = ["write", "--simulate", &cible.arg(), &source.arg()];
    let (code, plan, err) = scenario.run(&[&args[..], &["--dry-run"]].concat());
    assert_eq!((code, err.as_str()), (0, ""), "{plan}");
    assert!(!plan.contains("reread_table"), "{plan}");
    let (code, out, err) = scenario.run(&[&args[..], &["--yes", "--progress", "none"]].concat());
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        !scenario.log().iter().any(|call| call.starts_with("rescan")),
        "{:?}",
        scenario.log()
    );
}

/// Simulating onto the image itself would overwrite it while reading: a compressed image would be destroyed.
/// Refused before any open, plan included, whatever path names the file.
#[test]
fn une_simulation_vers_sa_propre_source_est_refusee() {
    let source = TempSource::new("meme-fichier", 4096);
    let avant = source.bytes();
    let scenario = Scenario::new();
    let chemin = std::path::PathBuf::from(source.arg());
    let detour = chemin
        .parent()
        .map(|dossier| {
            dossier
                .join(".")
                .join(chemin.file_name().unwrap_or_default())
        })
        .unwrap_or_default()
        .display()
        .to_string();
    let image = source.arg();
    for cible in [image.clone(), detour] {
        for fin in [&["--dry-run"][..], &["--yes", "--progress", "none"][..]] {
            let args = [
                &["write", "--simulate", cible.as_str(), image.as_str()][..],
                fin,
            ]
            .concat();
            let (code, out, err) = scenario.run(&args);
            assert_eq!(code, EXIT_USAGE, "{cible} {fin:?} : {out}");
            assert!(err.trim_end().ends_with("[E0001]"), "{err}");
        }
    }
    assert_eq!(source.bytes(), avant, "l'image a été touchée");
}

#[test]
fn sans_yes_rien_n_est_touche() {
    let source = TempSource::new("sans-yes", 4096);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["write", "usb-a", &source.arg()]);
    assert_eq!(code, EXIT_USAGE);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2004]"),
        "{err}"
    );
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn dry_run_decrit_sans_rien_faire() {
    let source = TempSource::new("dry", 4096);
    let scenario = Scenario::new();
    let (code, out, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Plan (nothing is written)") && out.contains("Unmount: /run/media/u/A"),
        "{out}"
    );
    assert!(
        out.contains("unmount, write, flush, verify, repair_gpt_if_needed, reread_table"),
        "{out}"
    );
    assert!(scenario.log().is_empty());
    let (_, json, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("JSON valide");
    assert_eq!(value["event"], "plan");
    assert_eq!(value["target"]["id"], "usb-a");
    assert_eq!(value["unmount"][0], "/run/media/u/A");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_disque_interdit_est_refuse_avant_toute_action() {
    let source = TempSource::new("forbidden", 4096);
    let scenario = Scenario::new();
    // Even with --yes and every option open.
    let (code, out, err) = scenario.run(&[
        "write",
        "nvme-sys",
        &source.arg(),
        "--yes",
        "--show-internal",
        "--allow-virtual",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2001]"),
        "{err}"
    );
    assert!(err.contains("system volume"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn seul_l_identifiant_stable_designe_une_cible() {
    let source = TempSource::new("chemin", 4096);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "/dev/sdb", &source.arg(), "--yes"]);
    assert_eq!(code, 1);
    assert!(err.trim_end().ends_with("[E1003]"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_disque_remplace_entre_les_deux_enumerations_n_est_pas_ecrit() {
    let source = TempSource::new("remplace", 4096);
    let mut scenario = Scenario::new();
    // First enumeration: the chosen key. Second, right before opening: another serial.
    scenario.enumerations = vec![vec![usb("S1")], vec![usb("S2")]];
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2002]"),
        "{err}"
    );
    // Unmounted, never opened.
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
    assert!(scenario.written(4096).iter().all(|b| *b == 0));
}

#[test]
fn un_disque_devenu_interdit_entre_les_deux_enumerations_n_est_pas_ecrit() {
    let source = TempSource::new("devenu", 4096);
    let mut scenario = Scenario::new();
    let mut now_system = usb("S1");
    now_system.partitions = vec![partition("/dev/sdb1", &["/"], true)];
    scenario.enumerations = vec![vec![usb("S1")], vec![now_system]];
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(err.trim_end().ends_with("[E2001]"), "{err}");
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
}

#[test]
fn echec_de_demontage_empeche_l_ouverture() {
    let source = TempSource::new("demontage", 4096);
    let mut scenario = Scenario::new();
    scenario.unmount = Err("could not unmount /run/media/u/A: target is busy".to_owned());
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, 1);
    assert!(
        err.contains("busy") && err.trim_end().ends_with("[E2003]"),
        "{err}"
    );
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
}

#[test]
fn droits_insuffisants_a_l_ouverture() {
    let source = TempSource::new("droits", 4096);
    let mut scenario = Scenario::new();
    scenario.open = Err(io::ErrorKind::PermissionDenied);
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, 1);
    assert!(err.trim_end().ends_with("[E4004]"), "{err}");
}

#[test]
fn image_connue_trop_grande_refusee_avant_le_demontage() {
    let source = TempSource::new("grande", DISK_SIZE + 512);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_SOURCE);
    assert!(err.trim_end().ends_with("[E3003]"), "{err}");
    assert!(
        scenario.log().is_empty(),
        "rien ne doit être démonté pour une image qui ne tient pas"
    );
}

#[test]
fn annulation_avant_toute_ecriture() {
    let source = TempSource::new("annule", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.cancel = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_CANCELLED);
    assert!(err.trim_end().ends_with("[E4003]"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}

#[test]
fn un_refus_d_autorisation_ne_demonte_rien() {
    let source = TempSource::new("refus", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.prepare = Err("authorization cancelled".to_owned());
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("[E4004]"), "{err}");
    assert!(err.contains("authorization cancelled"), "{err}");
    let log = scenario.log.borrow();
    assert_eq!(*log, ["prepare:/dev/sdb"], "ni démontage ni ouverture");
}

#[test]
fn annuler_pendant_l_autorisation_laisse_la_cle_montee() {
    let source = TempSource::new("annule-autorisation", 4096);
    let mut scenario = Scenario::new();
    scenario.cancel_on_prepare = true;
    let (code, _, _) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_CANCELLED);
    assert_eq!(scenario.log(), ["prepare:/dev/sdb"]);
}

#[test]
fn le_refus_d_elever_va_jusqu_au_demontage() {
    let source = TempSource::new("sans-elevation", 4096);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--elevate",
        "never",
    ]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(scenario.seen_elevation.get(), Some(Elevation::Never));
}

/// The file is replaced after opening: the hash covers the bytes that will be written, not what the path
/// names at check time.
#[test]
fn l_empreinte_porte_sur_le_fichier_ecrit_meme_remplace_entre_temps() {
    let ecrit = TempSource::from_bytes("piege-empreinte", &[0xAB; 1 << 20]);
    let attendu = TempSource::new("attendu-empreinte", 1 << 20);
    let mut lecteur = std::fs::File::open(&attendu.0).unwrap();
    let empreinte = bonk_core::write::hash_stream(
        &mut lecteur,
        None,
        bonk_core::write::DEFAULT_CHUNK,
        &mut |_| bonk_core::write::Control::Continue,
    )
    .unwrap();
    let (de, vers) = (attendu.0.clone(), ecrit.0.clone());
    let mut scenario = Scenario::new();
    scenario.on_first_enumeration = Some(Rc::new(move || {
        std::fs::rename(&de, &vers).unwrap();
    }));
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &ecrit.arg(),
        "--yes",
        "--mode",
        "raw",
        "--sha256",
        &empreinte,
    ]);
    assert_eq!(code, EXIT_SOURCE, "{out}\n{err}");
    assert!(err.contains("E3005"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}

/// A refused flush leaves the data in a cache that may never reach the drive: the write fails, even if the
/// read-back, served by that cache, looks right.
#[test]
fn un_vidage_refuse_fait_echouer_l_ecriture() {
    let source = TempSource::new("vidage", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.faults.flush = true;
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_ne!(code, 0, "{out}");
    assert!(err.contains("SYNCHRONIZE CACHE refused"), "{err}");
    assert!(
        err.trim_end().ends_with(']'),
        "code d'erreur attendu : {err}"
    );
}

/// Key pulled out mid-write: immediate failure, never a read-back or table repair on a missing drive, and a
/// message that says what happened.
#[test]
fn une_cle_debranchee_en_ecriture_arrete_tout() {
    let source = TempSource::new("debranchee", 8 << 20);
    let mut scenario = Scenario::new();
    scenario.faults.unplugged_after = Some(4 << 20);
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_ne!(code, 0, "{out}");
    assert!(
        err.contains("disappeared during the write") && err.contains("[E4001]"),
        "{err}"
    );
    assert!(
        err.trim_end().ends_with(']'),
        "code d'erreur attendu : {err}"
    );
    assert!(
        !scenario.log().iter().any(|entry| entry == "drop_cache"),
        "relecture tentée après le débranchement : {:?}",
        scenario.log()
    );
}

/// The ISO is written as published, then its empty first sector becomes an MBR naming its EFI image.
#[test]
fn une_iso_dvd_s_ecrit_hybride_et_seulement_sur_demande() {
    let iso = bonk_core::hybrid::testing::dvd_iso(128);
    let source = TempSource::from_bytes("dvd", &iso);
    let scenario = Scenario::new();
    let write = |mode: &str| {
        scenario.run(&[
            "write",
            "usb-a",
            &source.arg(),
            "--yes",
            "--mode",
            mode,
            "--progress",
            "none",
        ])
    };
    let (code, _, err) = write("auto");
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3006"), "{err}");
    let (code, out, err) = write("hybrid");
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("Hybrid MBR"), "{out}");
    let disk = scenario.written(iso.len());
    let expected = bonk_core::hybrid::hybrid_mbr(&iso).expect("MBR");
    assert_eq!(&disk[..512], expected.as_slice());
    assert_eq!(
        &disk[512..],
        &iso[512..],
        "le reste de l'ISO tel que publié"
    );
}

#[test]
fn une_iso_dont_le_premier_secteur_est_occupe_n_est_pas_rendue_hybride() {
    let mut iso = bonk_core::hybrid::testing::dvd_iso(128);
    iso[0] = 0xEB;
    let source = TempSource::from_bytes("dvd-occupee", &iso);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "hybrid",
        "--progress",
        "none",
    ]);
    assert_ne!(code, 0, "{err}");
    assert!(err.contains("first sector is not empty"), "{err}");
    assert!(
        scenario.log().is_empty(),
        "rien d'ouvert : {:?}",
        scenario.log()
    );
}
