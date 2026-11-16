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
    let sums = TempSource::from_bytes("sommes", format!("{sha}  image.img\n").as_bytes());
    let cache = TempSource::dir("cache-sommes");
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256-file",
        &sums.arg(),
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert_eq!(scenario.written(1 << 20), body);

    let (sums_url, _) = serve_file(format!("{sha}  image.img\n").into_bytes());
    let cache = TempSource::dir("cache-sommes-url");
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256-file",
        &sums_url,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""));

    let vide = TempSource::from_bytes("sommes-vides", b"# rien\n");
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256-file",
        &vide.arg(),
        "--cache-dir",
        &cache.arg(),
        "--yes",
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("image.img"), "{err}");
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
