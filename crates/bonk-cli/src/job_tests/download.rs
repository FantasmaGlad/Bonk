use super::*;

#[test]
fn une_adresse_se_telecharge_et_s_ecrit_puis_sert_le_cache() {
    let body = vec![0xCD; 1 << 20];
    let (url, hits) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let cache = TempSource::dir("cache-ok");
    let scenario = Scenario::new();
    let args = [
        "write",
        "usb-a",
        &url,
        "--sha256",
        &sha,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ];
    let (code, out, err) = scenario.run(&args);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert_eq!(scenario.written(1 << 20), body);
    let (code, _, err) = scenario.run(&args);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn une_empreinte_fausse_n_ecrit_rien_et_oublie_le_fichier() {
    let (url, hits) = serve_file(vec![0xCD; 1 << 20]);
    let faux = "0".repeat(64);
    let cache = TempSource::dir("cache-faux");
    let scenario = Scenario::new();
    let args = [
        "write",
        "usb-a",
        &url,
        "--sha256",
        &faux,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ];
    let (code, _, err) = scenario.run(&args);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3005"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
    let (code, _, _) = scenario.run(&args);
    assert_eq!(code, EXIT_SOURCE);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn une_adresse_sans_empreinte_ou_sans_confirmation_ne_telecharge_rien() {
    let (url, hits) = serve_file(vec![0; 16]);
    let cache = TempSource::dir("cache-refus");
    let scenario = Scenario::new();
    let (code, _, err) =
        scenario.run(&["write", "usb-a", &url, "--yes", "--cache-dir", &cache.arg()]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("--sha256"), "{err}");
    let faux = "0".repeat(64);
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256",
        &faux,
        "--cache-dir",
        &cache.arg(),
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");
    let (code, _, err) = scenario.run(&[
        "write",
        "absent",
        &url,
        "--sha256",
        &faux,
        "--yes",
        "--cache-dir",
        &cache.arg(),
    ]);
    assert_ne!(code, 0, "{err}");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn l_empreinte_se_lit_dans_un_fichier_d_empreintes_local_ou_distant() {
    let body = vec![0xEF; 1 << 20];
    let (url, _) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let line = format!("{sha}  image.img\n");
    let local = TempSource::from_bytes("sommes", line.as_bytes());
    let (distant, _) = serve_file(line.into_bytes());
    let vide = TempSource::from_bytes("sommes-vides", b"# rien\n");
    let scenario = Scenario::new();
    // (checksum file, expected exit code); an empty file names the image it misses.
    for (sums, attendu) in [(local.arg(), 0), (distant, 0), (vide.arg(), EXIT_USAGE)] {
        let cache = TempSource::dir("cache-sommes");
        let (code, out, err) = scenario.run(&[
            "write",
            "usb-a",
            &url,
            "--sha256-file",
            &sums,
            "--cache-dir",
            &cache.arg(),
            "--yes",
            "--mode",
            "raw",
            "--progress",
            "none",
        ]);
        assert_eq!(code, attendu, "{sums}: {out}{err}");
        if attendu == 0 {
            assert_eq!(err, "", "{sums}");
            assert_eq!(scenario.written(1 << 20), body);
        } else {
            assert!(err.contains("image.img"), "{err}");
        }
    }
}

#[test]
fn une_coupure_definitive_se_dit_par_son_code_sans_toucher_au_support() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/image.img",
        listener.local_addr().unwrap().port()
    );
    drop(listener);
    let faux = "0".repeat(64);
    let cache = TempSource::dir("cache-coupe");
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256",
        &faux,
        "--cache-dir",
        &cache.arg(),
        "--yes",
    ]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3008"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn le_flux_json_finit_par_un_evenement_d_erreur_avec_son_code() {
    let source = TempSource::new("flux-erreur", 4096);
    let faux = "0".repeat(64);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--sha256",
        &faux,
        "--progress",
        "json",
    ]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    let last: serde_json::Value = serde_json::from_str(out.lines().last().unwrap()).unwrap();
    assert_eq!(last["event"], "error", "{out}");
    assert_eq!(last["code"], "E3005", "{out}");
    assert!(err.contains("E3005"), "{err}");
}

#[test]
fn une_adresse_de_partage_se_garde_sous_le_nom_donne() {
    let body = vec![0xAB; 1 << 20];
    let (url, hits) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let scenario = Scenario::new();
    // (given name, exit code, file kept in the cache)
    for (nom, attendu, garde) in [
        ("GLF-OS-STABLE.iso", 0, Some("GLF-OS-STABLE.iso")),
        ("../evade.iso", EXIT_USAGE, None),
    ] {
        let cache = TempSource::dir("cache-nom");
        let (code, out, err) = scenario.run(&[
            "write",
            "usb-a",
            &url,
            "--sha256",
            &sha,
            "--file-name",
            nom,
            "--cache-dir",
            &cache.arg(),
            "--yes",
            "--mode",
            "raw",
            "--progress",
            "none",
        ]);
        assert_eq!(code, attendu, "{nom}: {out}{err}");
        if let Some(garde) = garde {
            let chemin = crate::cache::image_path(std::path::Path::new(&cache.arg()), &sha, garde);
            assert!(chemin.is_file(), "{}", chemin.display());
        } else {
            assert!(err.contains("one path segment"), "{err}");
        }
    }
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn bonk_info_lit_une_iso_distante_par_morceaux_et_dit_sa_famille_du_coffre() {
    let iso = bonk_core::iso9660::testing::image(
        "d-live 13",
        &[("live/vmlinuz", 1), ("live/initrd.img", 1)],
    );
    let (url, _) = serve_ranges(iso);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["info", &url]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains(&format!("Source: {url}")), "{out}");
    assert!(out.contains("Vault: Debian live, for x86_64 PCs"), "{out}");
    let (_, json, _) = scenario.run(&["info", &url, "--json"]);
    let report: serde_json::Value = serde_json::from_str(&json).expect("JSON");
    assert_eq!(report["vault"]["family"], "debian_live");
    assert_eq!(report["vault"]["arch"], "x86_64");
    assert!(scenario.log().is_empty(), "aucun disque touché");
}

#[test]
fn bonk_info_refuse_un_serveur_qui_renverrait_toute_l_image() {
    let (url, hits) = serve_file(vec![0; 1 << 20]);
    let (code, out, err) = Scenario::new().run(&["info", &url]);
    assert_ne!(code, 0);
    assert!(out.is_empty(), "{out}");
    assert!(
        err.contains("[E3008]") && err.contains("partial requests"),
        "{err}"
    );
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}
