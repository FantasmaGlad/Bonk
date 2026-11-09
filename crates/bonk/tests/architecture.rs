use std::fs;
use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("crates"), Path::to_path_buf)
}

/// `[dependencies]` lines of a crate, target-specific ones included, without dev-dependencies.
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
        (
            "bonk-ui",
            &["bonk-cli", "bonk-gui", "clap", "iced", "rfd", "libc"][..],
        ),
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

/// Production code of a file: its tests apart, as for the size cap.
fn production(file: &Path) -> String {
    // Windows checks out sources with CRLF line endings.
    let text = fs::read_to_string(file)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    text.split("#[cfg(test)]\nmod tests")
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// What gives away OS-specific code: platform attributes and direct OS calls.
const SYSTEM_MARKERS: &[&str] = &[
    "target_os",
    "target_family",
    "cfg(unix",
    "cfg(windows",
    "cfg(all(unix",
    "cfg(not(unix",
    "cfg(any(unix",
    "libc::",
    "std::os::",
];

/// Known exceptions, each with its reason; an exception no longer needed fails the test.
const SYSTEM_EXCEPTIONS: &[(&str, &[&str], &str)] = &[(
    "bonk-gui/src/lib.rs",
    &["target_os", "cfg(all(unix", "cfg(not("],
    "variables d'affichage d'une session graphique Unix hors macOS ; à déplacer dans bonk-hal (STATE.md, dette)",
)];

#[test]
fn le_code_propre_a_un_systeme_reste_dans_bonk_hal() {
    let mut used = vec![false; SYSTEM_EXCEPTIONS.len()];
    for name in [
        "bonk-core",
        "bonk-ops",
        "bonk-cli",
        "bonk-gui",
        "bonk-ui",
        "bonk-catalog",
    ] {
        let mut files = Vec::new();
        sources(&crates_dir().join(name).join("src"), &mut files);
        assert!(!files.is_empty(), "{name} : aucune source lue");
        for file in files {
            let text = production(&file);
            let relative = file
                .strip_prefix(crates_dir())
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            for marker in SYSTEM_MARKERS {
                if !text.contains(marker) {
                    continue;
                }
                let excepted = SYSTEM_EXCEPTIONS
                    .iter()
                    .position(|(path, allowed, _)| *path == relative && allowed.contains(marker));
                match excepted {
                    Some(index) => used[index] = true,
                    None => panic!(
                        "{relative} contient {marker} : à placer derrière bonk_hal::Platform"
                    ),
                }
            }
        }
    }
    for ((path, marker, _), used) in SYSTEM_EXCEPTIONS.iter().zip(used) {
        assert!(
            used,
            "exception sans objet, à retirer : {path} ({marker:?})"
        );
    }
}

/// A single write door: only the bonk-ops jobs and the system layer open a disk for writing. The window, the
/// shared screen, the catalog, the CLI and the Android app go through them.
#[test]
fn seule_la_porte_d_ecriture_ouvre_un_disque_en_ecriture() {
    for name in [
        "bonk-core",
        "bonk-ops",
        "bonk-cli",
        "bonk-gui",
        "bonk-ui",
        "bonk-catalog",
        "bonk-android",
    ] {
        let mut files = Vec::new();
        sources(&crates_dir().join(name).join("src"), &mut files);
        for file in files {
            let relative = file
                .strip_prefix(crates_dir())
                .unwrap_or(&file)
                .to_string_lossy()
                .replace('\\', "/");
            if relative.starts_with("bonk-ops/src/job/") {
                continue;
            }
            assert!(
                !production(&file).contains("Access::ReadWrite"),
                "{relative} ouvre un disque en écriture hors de bonk_ops::job"
            );
        }
    }
}

/// A file over this cap, tests apart, gets split: that is where catch-all files are born.
#[test]
fn aucun_fichier_de_production_ne_depasse_le_plafond() {
    const PLAFOND: usize = 700;
    fn parcourir(dossier: &std::path::Path, trouves: &mut Vec<(usize, String)>) {
        for entree in std::fs::read_dir(dossier).into_iter().flatten().flatten() {
            let chemin = entree.path();
            let nom = chemin.to_string_lossy().into_owned();
            if chemin.is_dir() {
                if !nom.ends_with("job_tests") && !nom.ends_with("target") {
                    parcourir(&chemin, trouves);
                }
            } else if nom.ends_with(".rs")
                && !nom.ends_with("tests.rs")
                && !chemin.components().any(|c| c.as_os_str() == "tests")
            {
                // Windows checks out sources with CRLF line endings.
                let texte = std::fs::read_to_string(&chemin)
                    .unwrap_or_default()
                    .replace("\r\n", "\n");
                let production = texte
                    .split("#[cfg(test)]\nmod tests")
                    .next()
                    .unwrap_or_default();
                let lignes = production.lines().count();
                if lignes > PLAFOND {
                    trouves.push((lignes, nom));
                }
            }
        }
    }
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut trouves = Vec::new();
    parcourir(&crates, &mut trouves);
    assert!(trouves.is_empty(), "{trouves:?}");
}
