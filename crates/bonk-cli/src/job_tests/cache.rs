use super::*;

#[test]
fn le_cache_se_liste_et_se_vide_sans_toucher_au_reste() {
    let body = vec![0xCD; 1 << 20];
    let (url, _) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let cache = TempSource::dir("cache-commandes");
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
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
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("bonk cache clean"), "{out}");

    let images = std::path::Path::new(&cache.arg()).join("images");
    std::fs::write(images.join("a-moi.txt"), b"garde").unwrap();
    std::fs::create_dir_all(images.join("autre")).unwrap();

    let (code, out, _) = scenario.run(&["cache", "list", "--cache-dir", &cache.arg(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["entries"].as_array().map(Vec::len), Some(1), "{out}");
    assert_eq!(value["entries"][0]["sha256"], sha);
    assert_eq!(value["total_bytes"], 1 << 20);

    let (code, out, _) =
        scenario.run(&["cache", "clean", "--cache-dir", &cache.arg(), "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(images.join(&sha).exists());

    let (code, out, _) = scenario.run(&["cache", "clean", "--cache-dir", &cache.arg()]);
    assert_eq!(code, 0, "{out}");
    assert!(!images.join(&sha).exists());
    assert!(images.join("a-moi.txt").exists());
    assert!(images.join("autre").exists());
}

#[test]
fn une_seule_image_du_cache_se_retire_et_chacune_a_sa_date() {
    let cache = TempSource::dir("cache-une");
    let images = std::path::Path::new(&cache.arg()).join("images");
    let (gardee, retiree) = ("a".repeat(64), "b".repeat(64));
    for (sha, name) in [(&gardee, "garde.iso"), (&retiree, "retire.iso")] {
        std::fs::create_dir_all(images.join(sha)).unwrap();
        std::fs::write(images.join(sha).join(name), b"image").unwrap();
    }
    let scenario = Scenario::new();

    let (code, out, _) = scenario.run(&["cache", "list", "--cache-dir", &cache.arg(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(value["entries"][0]["modified"].as_u64().is_some(), "{out}");

    let majuscules = retiree.to_ascii_uppercase();
    let (code, out, _) = scenario.run(&[
        "cache",
        "clean",
        &majuscules,
        "--cache-dir",
        &cache.arg(),
        "--json",
    ]);
    assert_eq!(code, 0, "{out}");
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["removed_bytes"], 5, "{out}");
    assert!(!images.join(&retiree).exists());
    assert!(images.join(&gardee).join("garde.iso").exists());

    let (code, _, err) = scenario.run(&[
        "cache",
        "clean",
        "pas-une-empreinte",
        "--cache-dir",
        &cache.arg(),
    ]);
    assert_ne!(code, 0);
    assert!(err.contains("invalid SHA-256"), "{err}");
    assert!(images.join(&gardee).exists());
}
