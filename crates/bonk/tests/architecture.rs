//! Les règles de dépendance du workspace, vérifiées plutôt que promises : une violation fait échouer les
//! tests au lieu de s'installer sans bruit.

use std::fs;
use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("crates"), Path::to_path_buf)
}

/// Lignes de la section `[dependencies]` d'un crate, cibles particulières comprises, sans les
/// dépendances de test.
fn dependencies(name: &str) -> Vec<String> {
    let text = fs::read_to_string(crates_dir().join(name).join("Cargo.toml")).unwrap_or_default();
    assert!(!text.is_empty(), "Cargo.toml de {name} illisible");
    let mut inside = false;
    let mut found = Vec::new();
    for line in text.lines() {
        if line.starts_with('[') {
            inside = line.ends_with("dependencies]") && !line.contains("dev-dependencies");
        } else if inside && let Some((key, _)) = line.split_once('=') {
            found.push(key.trim().trim_end_matches(".workspace").to_owned());
        }
    }
    found
}

/// Fichiers Rust de `src/`, récursivement.
fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            sources(&path, found);
        } else if path.extension().is_some_and(|e| e == "rs") {
            found.push(path);
        }
    }
}

#[test]
fn chaque_crate_ne_depend_que_de_ce_que_permet_sa_couche() {
    for (name, forbidden) in [
        (
            "bonk-core",
            &[
                "bonk-hal",
                "bonk-ops",
                "bonk-cli",
                "bonk-gui",
                "bonk-catalog",
                "bonk-customizer",
                "libc",
                "clap",
                "iced",
            ][..],
        ),
        (
            "bonk-hal",
            &["bonk-ops", "bonk-cli", "bonk-gui", "clap", "iced"][..],
        ),
        ("bonk-ops", &["bonk-cli", "bonk-gui", "clap", "iced"][..]),
        ("bonk-cli", &["bonk-gui", "iced"][..]),
        ("bonk-gui", &["bonk-cli", "clap", "libc"][..]),
    ] {
        let deps = dependencies(name);
        assert!(!deps.is_empty(), "{name} : aucune dépendance lue");
        for dep in forbidden {
            assert!(
                !deps.iter().any(|d| d == dep),
                "{name} ne doit pas dépendre de {dep}"
            );
        }
    }
}

#[test]
fn le_code_propre_a_un_systeme_reste_dans_bonk_hal() {
    for name in ["bonk-core", "bonk-ops", "bonk-cli", "bonk-gui"] {
        let mut files = Vec::new();
        sources(&crates_dir().join(name).join("src"), &mut files);
        assert!(!files.is_empty(), "{name} : aucune source lue");
        for file in files {
            let text = fs::read_to_string(&file).unwrap_or_default();
            for marker in ["cfg(target_os", "libc::"] {
                assert!(
                    !text.contains(marker),
                    "{} contient {marker} : à placer derrière bonk_hal::Platform",
                    file.display()
                );
            }
        }
    }
}
