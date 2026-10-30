//! Codes d'erreur stables et codes de sortie (CDC §9.1).
//!
//! Chaque erreur affichée se termine par son code entre crochets (`[E1003]`) : un outil le retrouve
//! sans interpréter le texte, et `bonk explain <code>` en donne le sens, les causes probables et le
//! remède. Les codes ne sont jamais réutilisés ni renumérotés.

use std::fmt::Display;
use std::io::{self, Write};

use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;

pub(crate) use bonk_ops::errors::*;

/// Écrit l'erreur sur `err` au format `bonk : erreur : <message> [Exxxx]` et rend le code de sortie.
pub(crate) fn report(err: &mut dyn Write, code: Code, message: impl Display) -> u8 {
    let _ = writeln!(err, "bonk : erreur : {message} [{}]", code.id());
    code.exit()
}

/// Explique un code d'erreur
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct ExplainArgs {
    /// Code à expliquer (`E1003`) ; sans code, liste tous les codes
    code: Option<String>,

    /// Sortie JSON
    #[arg(long)]
    json: bool,
}

#[derive(Serialize, JsonSchema)]
struct EntryJson {
    code: &'static str,
    exit_code: u8,
    title: &'static str,
    causes: &'static [&'static str],
    remedy: &'static str,
}

impl From<&Entry> for EntryJson {
    fn from(entry: &Entry) -> Self {
        Self {
            code: entry.id,
            exit_code: entry.exit,
            title: entry.title,
            causes: entry.causes,
            remedy: entry.remedy,
        }
    }
}

#[derive(Serialize, JsonSchema)]
struct ExplainReport {
    schema_version: u32,
    codes: Vec<EntryJson>,
}

impl ExplainArgs {
    pub(crate) fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ExplainReport)).unwrap_or_default()
}

/// Les fiches d'erreur en JSON, pour le service ; `None` si le code demandé n'existe pas.
pub(crate) fn explain_value(code: Option<&str>) -> Option<serde_json::Value> {
    let entries: Vec<EntryJson> = match code {
        Some(code) => vec![EntryJson::from(lookup(code)?)],
        None => CATALOG.iter().map(EntryJson::from).collect(),
    };
    serde_json::to_value(ExplainReport {
        schema_version: 1,
        codes: entries,
    })
    .ok()
}

/// `bonk explain`. Renvoie `false` si le code demandé n'existe pas.
pub(crate) fn explain(args: &ExplainArgs, out: &mut dyn Write) -> io::Result<bool> {
    let entries: Vec<&Entry> = match &args.code {
        Some(code) => match lookup(code) {
            Some(entry) => vec![entry],
            None => return Ok(false),
        },
        None => CATALOG.iter().collect(),
    };
    if args.json {
        let report = ExplainReport {
            schema_version: 1,
            codes: entries.iter().map(|e| EntryJson::from(*e)).collect(),
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        return writeln!(out).map(|()| true);
    }
    if args.code.is_none() {
        for entry in entries {
            writeln!(
                out,
                "{}  (sortie {})  {}",
                entry.id, entry.exit, entry.title
            )?;
        }
        return Ok(true);
    }
    for entry in entries {
        writeln!(out, "{} : {}", entry.id, entry.title)?;
        writeln!(out, "Code de sortie : {}", entry.exit)?;
        writeln!(out, "Causes probables :")?;
        for cause in entry.causes {
            writeln!(out, "  - {cause}")?;
        }
        writeln!(out, "Remède : {}", entry.remedy)?;
    }
    Ok(true)
}

/// Texte d'un code inconnu, avec la liste des codes existants.
pub(crate) fn unknown_code_message(text: &str) -> String {
    let ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
    format!("code inconnu : « {text} » (codes : {})", ids.join(", "))
}

#[cfg(test)]
mod tests {
    use bonk_hal::EnumerateError;

    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn identifiants_uniques_ordonnes_et_bien_formes() {
        let ids: Vec<&str> = CATALOG.iter().map(|e| e.id).collect();
        let unique: BTreeSet<&str> = ids.iter().copied().collect();
        assert_eq!(ids.len(), unique.len(), "identifiant en double : {ids:?}");
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "le catalogue suit l'ordre des numéros");
        for id in ids {
            assert!(
                id.len() == 5 && id.starts_with('E') && id[1..].bytes().all(|b| b.is_ascii_digit()),
                "{id}"
            );
        }
    }

    #[test]
    fn chaque_code_a_une_fiche_complete() {
        for entry in CATALOG {
            assert!(
                !entry.title.is_empty() && !entry.remedy.is_empty(),
                "{}",
                entry.id
            );
            assert!(!entry.causes.is_empty(), "{}", entry.id);
            assert_eq!(entry.code.entry().id, entry.id);
            assert!(
                (1..=6).contains(&entry.exit),
                "{} : sortie {}",
                entry.id,
                entry.exit
            );
        }
    }

    #[test]
    fn codes_de_sortie_par_classe() {
        assert_eq!(Code::TargetRefused.exit(), 3);
        assert_eq!(Code::IdentityChanged.exit(), 3);
        assert_eq!(Code::VerifyMismatch.exit(), 4);
        assert_eq!(Code::Cancelled.exit(), 5);
        assert_eq!(Code::SourceTooLarge.exit(), 6);
        assert_eq!(Code::ConfirmationRequired.exit(), 2);
        assert_eq!(Code::DiskNotFound.exit(), 1);
    }

    #[test]
    fn le_code_termine_le_message() {
        let mut err = Vec::new();
        let exit = report(&mut err, Code::DiskNotFound, "disque introuvable : « x »");
        assert_eq!(exit, EXIT_RUNTIME_ERROR);
        assert_eq!(
            String::from_utf8_lossy(&err),
            "bonk : erreur : disque introuvable : « x » [E1003]\n"
        );
    }

    #[test]
    fn recherche_insensible_a_la_casse() {
        assert_eq!(lookup("e1003").map(|e| e.id), Some("E1003"));
        assert_eq!(lookup(" E4002 ").map(|e| e.id), Some("E4002"));
        assert!(lookup("E9999").is_none());
    }

    #[test]
    fn explain_un_code_et_la_liste() -> io::Result<()> {
        let mut out = Vec::new();
        let args = ExplainArgs {
            code: Some("e2001".to_owned()),
            json: false,
        };
        assert!(explain(&args, &mut out)?);
        let text = String::from_utf8_lossy(&out).into_owned();
        assert!(
            text.starts_with("E2001 : Cible refusée par le filtre de sécurité"),
            "{text}"
        );
        assert!(text.contains("Code de sortie : 3"), "{text}");
        assert!(text.contains("bonk debug filter"), "{text}");

        let mut list = Vec::new();
        explain(
            &ExplainArgs {
                code: None,
                json: false,
            },
            &mut list,
        )?;
        let text = String::from_utf8_lossy(&list).into_owned();
        assert_eq!(text.lines().count(), CATALOG.len());
        assert!(text.contains("E4002  (sortie 4)"), "{text}");

        let mut json = Vec::new();
        explain(
            &ExplainArgs {
                code: Some("E4003".to_owned()),
                json: true,
            },
            &mut json,
        )?;
        let value: serde_json::Value = serde_json::from_slice(&json)?;
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["codes"][0]["exit_code"], 5);
        assert!(!explain(
            &ExplainArgs {
                code: Some("E0".to_owned()),
                json: false
            },
            &mut Vec::new()
        )?);
        Ok(())
    }

    #[test]
    fn echec_d_enumeration_vers_le_bon_code() {
        assert_eq!(
            Code::of_enumeration(&EnumerateError::Unsupported { os: "x" }),
            Code::EnumerationUnsupported
        );
        let malformed = EnumerateError::Malformed {
            what: "a".into(),
            detail: "b".into(),
        };
        assert_eq!(Code::of_enumeration(&malformed), Code::EnumerationFailed);
    }
}
