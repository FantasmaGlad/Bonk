//! Aides partagées par les tests de processus. Chaque binaire de test n'en utilise qu'une partie.
#![allow(dead_code)]

use std::process::Command;

/// Vrai si l'environnement est celui d'une intégration continue (GitHub Actions et la plupart des autres
/// posent `CI`).
pub fn en_ci() -> bool {
    std::env::var_os("CI").is_some()
}

/// `command -v` : l'outil est-il trouvable dans le `PATH` ?
fn installe(nom: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {nom}")])
        .output()
        .is_ok_and(|s| s.status.success())
}

/// Décision pure, testable sans toucher à l'environnement : un outil absent saute le test hors CI (en le
/// disant) et le fait échouer en CI, où l'exécuteur est figé (`ci.yml`) et où un saut ferait passer pour
/// vérifié ce qui ne l'a pas été.
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

/// L'outil est-il disponible ? Faux (avec message) hors CI s'il manque ; panique en CI.
pub fn outil_disponible(nom: &str) -> bool {
    decider(nom, installe(nom), en_ci())
}

/// Un `xvfb-run` installé ne prouve pas qu'un serveur X démarre (poste sans `Xvfb` ou sans `xauth`).
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

/// Statut, sortie standard et sortie d'erreur d'un processus terminé, pour les messages d'assertion : un
/// échec en CI doit se lire sans relancer le test.
pub fn diagnostic(sortie: &std::process::Output) -> String {
    format!(
        "statut : {:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        sortie.status,
        String::from_utf8_lossy(&sortie.stdout),
        String::from_utf8_lossy(&sortie.stderr)
    )
}

/// Vérifie le code de sortie ; l'échec affiche le diagnostic complet.
#[track_caller]
pub fn assert_code(sortie: &std::process::Output, attendu: i32) {
    assert_eq!(
        sortie.status.code(),
        Some(attendu),
        "{}",
        diagnostic(sortie)
    );
}
