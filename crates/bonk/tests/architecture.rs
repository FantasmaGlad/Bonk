use std::fs;
use std::path::{Path, PathBuf};

fn crates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_else(|| PathBuf::from("crates"), Path::to_path_buf)
}

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

fn read(file: &Path) -> String {
    // Windows checks out sources with CRLF line endings.
    fs::read_to_string(file)
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

fn module_of(file: &Path) -> Option<(String, PathBuf)> {
    let dir = file.parent()?;
    let stem = file.file_stem()?.to_str()?;
    if stem == "mod" {
        Some((
            dir.file_name()?.to_str()?.to_owned(),
            dir.parent()?.to_owned(),
        ))
    } else {
        Some((stem.to_owned(), dir.to_owned()))
    }
}

fn test_only(file: &Path) -> bool {
    let Some((name, home)) = module_of(file) else {
        return false;
    };
    let mut parents = vec![
        home.join("mod.rs"),
        home.join("lib.rs"),
        home.join("main.rs"),
    ];
    if let (Some(up), Some(dir)) = (home.parent(), home.file_name()) {
        parents.push(up.join(dir).with_extension("rs"));
    }
    parents
        .iter()
        .filter(|parent| parent.as_path() != file)
        .any(|parent| {
            declaration(&read(parent), &name)
                .is_some_and(|under_test| under_test || test_only(parent))
        })
}

fn declaration(text: &str, name: &str) -> Option<bool> {
    let wanted = format!("mod {name};");
    let mut previous = "";
    for line in text.lines().map(str::trim) {
        if line
            .strip_suffix(wanted.as_str())
            .is_some_and(|visibility| visibility.is_empty() || visibility.starts_with("pub"))
        {
            return Some(previous == "#[cfg(test)]");
        }
        previous = line;
    }
    None
}

fn production(file: &Path) -> String {
    if test_only(file) {
        return String::new();
    }
    read(file)
        .split("#[cfg(test)]\nmod tests {")
        .next()
        .unwrap_or_default()
        .to_owned()
}

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
    "variables d'affichage d'une session graphique Unix et réception du lien bonk:// sous macOS, lues avant l'ouverture de la fenêtre",
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
                    .split("#[cfg(test)]\nmod tests {")
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

#[test]
fn un_module_de_tests_a_part_n_est_pas_du_code_de_production() {
    let cli = crates_dir().join("bonk-cli").join("src");
    assert!(test_only(&cli.join("config").join("tests.rs")));
    assert!(test_only(&cli.join("job_tests").join("write.rs")));
    assert!(test_only(&cli.join("job_tests").join("vault.rs")));
    let catalog = crates_dir()
        .join("bonk-catalog")
        .join("src")
        .join("manifest");
    assert!(production(&catalog.join("mod.rs")).contains("DEFAULT_BASE"));
    assert!(!test_only(&cli.join("config").join("store.rs")));
    assert!(!test_only(&cli.join("lib.rs")));
}

#[test]
fn l_adresse_du_catalogue_n_est_ecrite_qu_une_fois() {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default();
            if path.is_dir() {
                if name != "target" && name != "build" {
                    walk(&path, found);
                }
            } else if path.extension().is_some_and(|e| e == "rs" || e == "kt") {
                found.push(path);
            }
        }
    }
    let crates = crates_dir();
    let mut files = Vec::new();
    walk(&crates, &mut files);
    walk(
        &crates.join("..").join("android").join("app").join("src"),
        &mut files,
    );
    let definition = crates
        .join("bonk-catalog")
        .join("src")
        .join("manifest")
        .join("mod.rs");
    let host = ["catalog", "bonk-os", "app"].join(".");
    let written: Vec<PathBuf> = files
        .into_iter()
        .filter(|file| {
            *file != definition
                && !file.ends_with("tests/architecture.rs")
                && read(file).contains(&host)
        })
        .collect();
    assert!(written.is_empty(), "{written:?}");
    assert!(read(&definition).contains(&host));
}

/// Every host written in production code, with its use; image hosts come only from the signed catalog or the user.
const KNOWN_HOSTS: &[(&str, &str)] = &[
    ("catalog.bonk-os.app", "catalog, its signature and logos"),
    (
        "www.torproject.org",
        "link shown to the user, never fetched by Bonk",
    ),
];

fn written_hosts(text: &str) -> Vec<String> {
    let mut hosts = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with("//")) {
        for (start, _) in line.match_indices("://") {
            if !(line[..start].ends_with("http") || line[..start].ends_with("https")) {
                continue;
            }
            let host: String = line[start + 3..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-')
                .collect();
            if host.contains('.') && !host.ends_with('.') {
                hosts.push(host);
            }
        }
    }
    hosts
}

#[test]
fn seuls_les_hotes_connus_sont_ecrits_dans_le_code() {
    let crates = crates_dir();
    let mut rust = Vec::new();
    sources(&crates, &mut rust);
    let mut texts: Vec<(PathBuf, String)> = rust
        .into_iter()
        .filter(|file| !file.components().any(|part| part.as_os_str() == "tests"))
        .map(|file| (file.clone(), production(&file)))
        .collect();
    let kotlin = crates
        .join("..")
        .join("android")
        .join("app")
        .join("src")
        .join("main");
    let mut stack = vec![kotlin];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "kt") {
                texts.push((path.clone(), read(&path)));
            }
        }
    }
    assert!(
        texts
            .iter()
            .any(|(file, _)| file.extension().is_some_and(|e| e == "kt"))
    );
    let unknown: Vec<String> = texts
        .iter()
        .flat_map(|(file, text)| {
            written_hosts(text)
                .into_iter()
                .map(move |host| (host, file))
        })
        .filter(|(host, _)| !KNOWN_HOSTS.iter().any(|(known, _)| known == host))
        .map(|(host, file)| format!("{host} : {}", file.display()))
        .collect();
    assert!(
        unknown.is_empty(),
        "hôte absent de KNOWN_HOSTS : {unknown:#?}"
    );
}

#[test]
fn une_adresse_ecrite_donne_son_hote() {
    let text = "let a = \"https://a.example.org/x\";\n// https://b.org\nhttps:// www. ftp://c.org";
    assert_eq!(written_hosts(text), ["a.example.org"]);
    assert_eq!(
        written_hosts("trim(\"https://www.\")"),
        Vec::<String>::new()
    );
}
