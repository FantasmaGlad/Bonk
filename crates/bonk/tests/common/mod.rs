#![allow(dead_code)]

use std::process::Command;

/// GitHub Actions and most others set `CI`.
pub fn en_ci() -> bool {
    std::env::var_os("CI").is_some()
}

fn installe(nom: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {nom}")])
        .output()
        .is_ok_and(|s| s.status.success())
}

/// Pure decision, testable without the environment: a missing tool skips the test outside CI (saying so) and
/// fails it in CI, where the runner is fixed (`ci.yml`) and a skip would pass for a check that never ran.
pub fn decider(nom: &str, present: bool, ci: bool) -> bool {
    if present {
        return true;
    }
    assert!(
        !ci,
        "{nom} est requis en CI mais absent : l'ajouter à l'image ou aux paquets de ci.yml"
    );
    eprintln!("{nom} absent : test ignoré");
    false
}

/// False (with a message) outside CI when missing; panics in CI.
pub fn outil_disponible(nom: &str) -> bool {
    decider(nom, installe(nom), en_ci())
}

/// An installed `xvfb-run` does not prove an X server starts (machine without `Xvfb` or `xauth`).
pub fn affichage_virtuel_disponible() -> bool {
    if !outil_disponible("xvfb-run") || !outil_disponible("timeout") {
        return false;
    }
    let demarre = Command::new("xvfb-run")
        .args(["-a", "true"])
        .output()
        .is_ok_and(|s| s.status.success());
    decider("un serveur X sous xvfb-run", demarre, en_ci())
}

/// Status, stdout and stderr of a finished process, for assertion messages: a CI failure must be readable
/// without rerunning the test.
pub fn diagnostic(sortie: &std::process::Output) -> String {
    format!(
        "statut : {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        sortie.status,
        String::from_utf8_lossy(&sortie.stdout),
        String::from_utf8_lossy(&sortie.stderr)
    )
}

#[track_caller]
pub fn assert_code(sortie: &std::process::Output, attendu: i32) {
    assert_eq!(
        sortie.status.code(),
        Some(attendu),
        "{}",
        diagnostic(sortie)
    );
}
