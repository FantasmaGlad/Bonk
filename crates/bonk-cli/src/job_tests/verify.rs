use super::*;

#[test]
fn une_relecture_fausse_fait_echouer_la_verification() {
    let source = TempSource::new("defectueux", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.faults.reads = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
    assert!(
        err.contains("first wrong byte at offset 0 (MiB 0)"),
        "{err}"
    );
    let (code, _, _) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
        "--no-verify",
    ]);
    assert_eq!(code, 0);
}

#[test]
fn la_commande_verify_compare_sans_ecrire() {
    let source = TempSource::new("verify", 1 << 20);
    let scenario = Scenario::new();
    scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    scenario.log.borrow_mut().clear();
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("Verification passed"), "{out}");
    assert_eq!(
        scenario.log(),
        ["prepare:/dev/sdb", "open:/dev/sdb:ReadOnly", "drop_cache"]
    );
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(0, &[0xAA; 512])
        .expect("écriture de test");
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
}

#[test]
fn verify_json_et_progression_json() {
    let source = TempSource::new("json", 2 << 20);
    let scenario = Scenario::new();
    let (code, out, _) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "json",
    ]);
    assert_eq!(code, 0);
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
        .collect();
    let (last, progress) = lines.split_last().expect("des lignes");
    assert_eq!(last["event"], "result");
    assert_eq!(last["write"]["bytes"], 2 << 20);
    assert!(
        progress.iter().all(|e| e["event"] == "progress"),
        "{progress:?}"
    );
    assert!(
        progress.iter().any(|e| e["phase"] == "write")
            && progress.iter().any(|e| e["phase"] == "verify")
    );
    let done: Vec<u64> = progress
        .iter()
        .filter(|e| e["phase"] == "write")
        .map(|e| e["bytes_done"].as_u64().unwrap_or(0))
        .collect();
    assert!(done.windows(2).all(|w| w[0] <= w[1]), "{done:?}");
}

#[test]
fn verify_d_une_image_gpt_reussit_apres_la_reparation_du_gpt() {
    use bonk_core::partition::testing::{LINUX, gpt_image};

    let image = gpt_image(2048, &[(64, 1023, LINUX, "donnees")]);
    let source = TempSource::from_bytes("gpt-verify", &image);
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
    assert!(out.contains("GPT: repaired"), "{out}");
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(4096, &[0xAA; 512])
        .expect("écriture de test");
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
    assert!(
        err.contains("first wrong byte at offset 4096 (MiB 0)"),
        "{err}"
    );
}

#[test]
fn une_relecture_fausse_puis_juste_dit_que_le_support_est_instable() {
    let source = TempSource::new("instable", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.faults.first_read = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.contains("returns unstable data"), "{err}");
}

/// Without dropping the cache before the read-back, the check reads the kernel, not the drive (2,193 MB/s
/// measured on a USB 2.0 key): a write the key lost would go unseen.
#[test]
fn la_relecture_de_controle_ne_lit_jamais_le_cache() {
    let source = TempSource::new("cache", 1 << 20);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
        "--no-repair-gpt",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        !scenario
            .log()
            .contains(&"lecture_avant_vidage_du_cache".to_owned()),
        "{:?}",
        scenario.log()
    );
    scenario.log.borrow_mut().clear();
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        scenario.log(),
        ["prepare:/dev/sdb", "open:/dev/sdb:ReadOnly", "drop_cache"]
    );
}

#[test]
fn sans_reparation_demandee_le_gpt_n_est_pas_dit_sain() {
    let source = TempSource::new("sans-gpt", 4096);
    let (code, out, err) =
        Scenario::new().run(&["write", "usb-a", &source.arg(), "--yes", "--no-repair-gpt"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("GPT: check not requested"), "{out}");
    assert!(!out.contains("nothing to repair"), "{out}");
}

#[test]
fn la_verification_apres_reparation_dit_ce_que_couvre_son_empreinte() {
    use bonk_core::format::table::{TableKind, write_table};
    use bonk_core::format::{Filesystem, Ids};
    let mut image = MemoryDevice::new(8 << 20, 512).expect("géométrie valide");
    write_table(
        &mut image,
        TableKind::Gpt,
        Filesystem::Fat32,
        None,
        &Ids::from_seed(b"img"),
        false,
    )
    .expect("table");
    let source = TempSource::from_bytes("gpt-a-reparer", image.as_bytes());
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("Excluded from the comparison"), "{out}");
    assert!(
        out.contains("differs from the image's own hash"),
        "l'empreinte doit dire qu'elle n'est pas celle de l'image : {out}"
    );
}

#[test]
fn une_iso_hybride_garde_sa_gpt_telle_que_publiee() {
    use bonk_core::partition::testing::{ESP, gpt_image};

    let mut iso = gpt_image(2048, &[(128, 1023, ESP, "EFI")]);
    let primary = &mut iso[16 * 2048..17 * 2048];
    primary[0] = 1;
    primary[1..6].copy_from_slice(b"CD001");
    primary[80..84].copy_from_slice(&512u32.to_le_bytes());
    primary[128..130].copy_from_slice(&2048u16.to_le_bytes());
    iso.extend(vec![0u8; 300 * 1024]);
    let source = TempSource::from_bytes("iso-gpt", &iso);
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
    assert!(out.contains("Image: hybrid ISO"), "{out}");
    assert!(out.contains("GPT: kept as published, as an ISO"), "{out}");
    assert_eq!(
        &scenario.written(iso.len())[..],
        &iso[..],
        "l'ISO telle que publiée"
    );
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    let (_, plan, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert!(!plan.contains("repair_gpt_if_needed"), "{plan}");
}
