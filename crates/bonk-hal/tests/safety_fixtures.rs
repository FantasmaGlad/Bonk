//! Le filtre de sécurité face aux jeux d'énumération de `tests/fixtures/`.
//!
//! Chaque fichier décrit des disques et, pour chaque jeu de réglages, le verdict attendu de **chacun**
//! : un disque sans verdict attendu fait échouer le test, pour qu'aucun cas ne soit ajouté sans être
//! vérifié. Le verdict réel est comparé en JSON, donc ce test fige aussi le format des verdicts que
//! rend `bonk list --json`.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fs;
use std::path::PathBuf;

use bonk_hal::{Disk, FilterOptions, evaluate, evaluate_traced};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    // Lu pour être validé par `deny_unknown_fields`, utile au lecteur du fichier seulement.
    #[allow(dead_code)]
    description: String,
    disks: Vec<Disk>,
    scenarios: Vec<Scenario>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    name: String,
    #[serde(default)]
    options: FilterOptions,
    /// Verdicts attendus, par identifiant de disque.
    expected: Option<BTreeMap<String, Value>>,
    /// Reprend les verdicts d'un autre jeu : les réglages ouverts ne doivent pas lever une exclusion.
    same_as: Option<String>,
}

fn fixture_paths() -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn check_fixture(path: &PathBuf) -> Result<Vec<String>, Box<dyn Error>> {
    let fixture: Fixture = serde_json::from_str(&fs::read_to_string(path)?)?;
    let file = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut failures = Vec::new();

    let ids: BTreeSet<&str> = fixture.disks.iter().map(|d| d.id.as_str()).collect();
    if ids.len() != fixture.disks.len() {
        failures.push(format!("{file} : identifiants de disque en double"));
    }

    for scenario in &fixture.scenarios {
        let label = format!("{file} / {}", scenario.name);
        let expected = match (&scenario.expected, &scenario.same_as) {
            (Some(map), None) => map,
            (None, Some(other)) => {
                let found = fixture
                    .scenarios
                    .iter()
                    .find(|s| &s.name == other)
                    .and_then(|s| s.expected.as_ref());
                match found {
                    Some(map) => map,
                    None => {
                        failures.push(format!("{label} : same_as \"{other}\" introuvable"));
                        continue;
                    }
                }
            }
            _ => {
                failures.push(format!("{label} : exactement un de expected ou same_as"));
                continue;
            }
        };

        let expected_ids: BTreeSet<&str> = expected.keys().map(String::as_str).collect();
        if expected_ids != ids {
            failures.push(format!(
                "{label} : verdicts attendus et disques différents (disques {ids:?}, attendus {expected_ids:?})"
            ));
        }

        for disk in &fixture.disks {
            let verdict = evaluate(disk, scenario.options);
            // L'examen règle par règle donne toujours le même verdict que le filtre lui-même.
            let (traced, trace) = evaluate_traced(disk, scenario.options);
            if traced != verdict || trace.is_empty() {
                failures.push(format!(
                    "{label} / {} : évaluation tracée différente",
                    disk.id
                ));
            }
            let actual = serde_json::to_value(verdict)?;
            match expected.get(&disk.id) {
                Some(want) if want != &actual => {
                    failures.push(format!(
                        "{label} / {} : attendu {want}, obtenu {actual}",
                        disk.id
                    ));
                }
                _ => {}
            }
        }
    }
    Ok(failures)
}

#[test]
fn verdicts_conformes_aux_jeux_d_enumeration() -> Result<(), Box<dyn Error>> {
    let paths = fixture_paths()?;
    assert!(paths.len() >= 5, "jeux trouvés : {paths:?}");
    let mut failures = Vec::new();
    for path in &paths {
        failures.extend(check_fixture(path)?);
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    Ok(())
}
