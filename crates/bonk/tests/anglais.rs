//! La ligne de commande parle anglais, quelles que soient la commande et l'issue : un script et le support
//! comparent ces textes, et rien ne les traduit.

use std::process::Command;

const COMMANDES: &[&[&str]] = &[
    &["--help"],
    &["disk", "--help"],
    &["disk", "list", "--help"],
    &["disk", "info", "--help"],
    &["disk", "write", "--help"],
    &["disk", "verify", "--help"],
    &["disk", "repair-gpt", "--help"],
    &["catalog", "--help"],
    &["catalog", "list", "--help"],
    &[
        "catalog",
        "list",
        "--catalog-url",
        "http://127.0.0.1:1/",
        "--cache-dir",
        "missing-cache",
    ],
    &[
        "write",
        "x",
        "--image",
        "missing",
        "--catalog-url",
        "http://127.0.0.1:1/",
        "--cache-dir",
        "missing-cache",
    ],
    &["list", "--proxy", "http://127.0.0.1:1"],
    &["cache", "--help"],
    &["cache", "list", "--help"],
    &["cache", "clean", "--help"],
    &["list", "--help"],
    &["info", "--help"],
    &["write", "--help"],
    &["verify", "--help"],
    &["repair-gpt", "--help"],
    &["format", "--help"],
    &["capabilities", "--help"],
    &["diagnose", "--help"],
    &["debug", "--help"],
    &["debug", "sources", "--help"],
    &["debug", "filter", "--help"],
    &["debug", "layout", "--help"],
    &["debug", "hexdump", "--help"],
    &["explain", "--help"],
    &["config", "--help"],
    &["config", "list", "--help"],
    &["config", "get", "--help"],
    &["config", "set", "--help"],
    &["config", "reset", "--help"],
    &["schema", "--help"],
    &["completions", "--help"],
    &["serve", "--help"],
    &["gui", "--help"],
    &["--no-gui"],
    &["explain"],
    &["capabilities"],
    &["config", "list"],
    &["info", "missing-disk"],
    &["config", "get", "missing-setting"],
    &["explain", "E9999"],
    &["schema", "missing-schema"],
    &["write", "missing-image.iso", "missing-disk", "--yes"],
    &["write", "--simulate", "target.img", "missing-image.iso"],
    &["debug", "hexdump", "missing-image.bin"],
    &["debug", "layout", "missing-image.bin"],
];

/// Mots français courants, sans équivalent anglais de même graphie. Les accents se voient à part.
const MOTS: &[&str] = &[
    "le",
    "la",
    "les",
    "un",
    "une",
    "des",
    "du",
    "de",
    "et",
    "ou",
    "sans",
    "avec",
    "pour",
    "dans",
    "sur",
    "aucun",
    "aucune",
    "disque",
    "disques",
    "fichier",
    "octets",
    "cible",
    "erreur",
    "introuvable",
    "inconnu",
    "inconnue",
    "oui",
    "non",
];

fn francais(texte: &str) -> Option<String> {
    for ligne in texte.lines() {
        if !ligne.is_ascii() {
            return Some(ligne.to_owned());
        }
        let minuscule = ligne.to_lowercase();
        let mots: Vec<&str> = minuscule
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .collect();
        if MOTS.iter().any(|m| mots.contains(m)) {
            return Some(ligne.to_owned());
        }
    }
    None
}

#[test]
fn toutes_les_sorties_sont_en_anglais() -> Result<(), Box<dyn std::error::Error>> {
    let dossier = std::env::temp_dir().join(format!("bonk-anglais-{}", std::process::id()));
    std::fs::create_dir_all(&dossier)?;
    let mut fautes = Vec::new();
    for arguments in COMMANDES {
        let sortie = Command::new(env!("CARGO_BIN_EXE_bonk"))
            .args(*arguments)
            .current_dir(&dossier)
            .env("BONK_CONFIG", dossier.join("config.json"))
            .output()?;
        for flux in [&sortie.stdout, &sortie.stderr] {
            if let Some(ligne) = francais(&String::from_utf8_lossy(flux)) {
                fautes.push(format!("bonk {} : {ligne}", arguments.join(" ")));
            }
        }
    }
    std::fs::remove_dir_all(&dossier)?;
    assert!(fautes.is_empty(), "{}", fautes.join("\n"));
    Ok(())
}

#[test]
fn le_detecteur_reconnait_le_francais() {
    assert!(francais("Liste les disques").is_some());
    assert!(francais("write failed: voilà").is_some());
    assert!(francais("List the disks Bonk can write").is_none());
}
