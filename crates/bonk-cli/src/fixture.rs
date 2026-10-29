//! `--fixture <fichier>` : rejouer une capture d'énumération (CDC §9.4).
//!
//! La capture est produite par `bonk diagnose --capture` sur la machine à analyser ; elle se rejoue sur
//! une autre, qui ne touche alors à aucun disque. Pour que rejouer ne puisse jamais écrire sur un vrai
//! disque, `--fixture` n'autorise que les commandes de lecture, `--dry-run` et `--simulate`.

use std::fs;
use std::path::{Path, PathBuf};

use bonk_hal::SourceRead;
use clap::Args;
use serde::Deserialize;

/// Options globales liées aux captures.
#[derive(Debug, Args)]
pub(crate) struct GlobalArgs {
    /// Rejoue une énumération capturée (`bonk diagnose --capture`) au lieu de lire ce système :
    /// lecture, `--dry-run` et `--simulate` seulement, jamais d'écriture sur un disque
    #[arg(long, value_name = "FICHIER", global = true)]
    fixture: Option<PathBuf>,
}

impl GlobalArgs {
    pub(crate) fn path(&self) -> Option<&Path> {
        self.fixture.as_deref()
    }
}

#[derive(Deserialize)]
struct Capture {
    reads: Vec<SourceRead>,
}

#[derive(Deserialize)]
struct Document {
    capture: Option<Capture>,
    reads: Option<Vec<SourceRead>>,
}

/// Lit les lectures d'un rapport `diagnose --capture` (ou d'une capture seule).
pub(crate) fn load(path: &Path) -> Result<Vec<SourceRead>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("{} : lecture impossible : {e}", path.display()))?;
    parse(&text).map_err(|e| format!("{} : {e}", path.display()))
}

pub(crate) fn parse(text: &str) -> Result<Vec<SourceRead>, String> {
    let document: Document = serde_json::from_str(text)
        .map_err(|e| format!("pas une capture de Bonk (JSON illisible : {e})"))?;
    let reads = document.capture.map(|c| c.reads).or(document.reads).ok_or(
        "pas de capture dans ce fichier (produire le rapport avec `bonk diagnose --capture`)",
    )?;
    if reads.is_empty() {
        return Err("capture vide".to_owned());
    }
    Ok(reads)
}

#[cfg(test)]
mod tests {
    use super::*;

    const READ: &str = r#"{"path":"/proc/swaps","kind":"file","ok":true,"bytes":3,"error":null,"micros":1,"content":"x\n"}"#;

    #[test]
    fn accepte_un_rapport_et_une_capture_seule() {
        let report = format!(r#"{{"schema_version":1,"capture":{{"reads":[{READ}]}}}}"#);
        assert_eq!(parse(&report).map(|r| r.len()), Ok(1));
        let bare = format!(r#"{{"reads":[{READ}]}}"#);
        assert_eq!(
            parse(&bare).map(|r| r[0].path.clone()),
            Ok("/proc/swaps".to_owned())
        );
    }

    #[test]
    fn refuse_ce_qui_n_est_pas_une_capture() {
        assert!(parse("pas du json").is_err_and(|e| e.contains("JSON illisible")));
        assert!(parse(r#"{"schema_version":1}"#).is_err_and(|e| e.contains("diagnose --capture")));
        assert!(parse(r#"{"reads":[]}"#).is_err_and(|e| e.contains("vide")));
        assert!(parse(r#"{"reads":[{"path":1}]}"#).is_err());
    }

    #[test]
    fn fichier_absent() {
        assert!(
            load(Path::new("/n/existe/pas.json")).is_err_and(|e| e.contains("lecture impossible"))
        );
    }
}
