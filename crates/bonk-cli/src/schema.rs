//! `bonk schema <commande>` : le JSON Schema de la sortie `--json` d'une commande (CDC §9.1).
//!
//! Les schémas sont générés à partir des types qui produisent la sortie : ils ne peuvent pas diverger
//! d'elle, et un test vérifie que chaque schéma existe et décrit un objet.

use std::io::{self, Write};

use clap::Args;
use serde_json::Value;

use crate::{config, debug, diagnose, errors, info, list, transfer};

/// Nom d'une commande et générateur du schéma de sa sortie.
type Entry = (&'static str, fn() -> Value);

/// Les schémas disponibles.
pub(crate) fn registry() -> Vec<Entry> {
    vec![
        ("list", list::schema),
        ("info", info::schema),
        ("diagnose", diagnose::schema),
        ("explain", errors::schema),
        ("config", config::schema),
        ("debug-filter", debug::schema_filter),
        ("debug-sources", debug::schema_sources),
        ("debug-layout", debug::schema_layout),
        ("write", transfer::schema_write),
        ("write-plan", transfer::schema_plan),
        ("verify", transfer::schema_verify),
        ("repair-gpt", transfer::schema_repair),
        ("progress", transfer::schema_progress),
    ]
}

/// Schéma JSON de la sortie d'une commande
#[derive(Debug, Args)]
// `long_about = None` : sans lui, clap reprendrait le commentaire ci-dessus comme texte long.
#[command(long_about = None)]
pub(crate) struct SchemaArgs {
    /// Commande dont on veut le schéma ; sans nom, liste les commandes
    name: Option<String>,
}

impl SchemaArgs {
    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// Écrit le schéma demandé (ou la liste). Renvoie `false` si le nom est inconnu.
pub(crate) fn run(args: &SchemaArgs, out: &mut dyn Write) -> io::Result<bool> {
    let registry = registry();
    match &args.name {
        None => {
            for (name, _) in &registry {
                writeln!(out, "{name}")?;
            }
            Ok(true)
        }
        Some(name) => {
            let Some((_, make)) = registry.iter().find(|(n, _)| n == name) else {
                return Ok(false);
            };
            serde_json::to_writer_pretty(&mut *out, &make()).map_err(io::Error::from)?;
            writeln!(out)?;
            Ok(true)
        }
    }
}

pub(crate) fn unknown_message(name: &str) -> String {
    let names: Vec<&str> = registry().iter().map(|(n, _)| *n).collect();
    format!(
        "schéma inconnu : « {name} » (schémas : {})",
        names.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_schema_decrit_un_objet_versionne() {
        for (name, make) in registry() {
            let schema = make();
            assert_eq!(schema["type"], "object", "{name}");
            let properties = schema["properties"].as_object();
            assert!(
                properties.is_some_and(|p| p.contains_key("schema_version")),
                "{name} : pas de schema_version"
            );
        }
    }

    #[test]
    fn le_schema_de_list_decrit_les_champs_reels() {
        let schema = list::schema();
        let text = schema.to_string();
        for field in [
            "eligibility",
            "partitions",
            "serial",
            "size_bytes",
            "verdict",
        ] {
            assert!(text.contains(field), "{field} absent du schéma de list");
        }
    }

    #[test]
    fn liste_inconnu_et_sortie() -> io::Result<()> {
        let mut out = Vec::new();
        assert!(run(&SchemaArgs { name: None }, &mut out)?);
        assert!(
            String::from_utf8_lossy(&out)
                .lines()
                .any(|l| l == "diagnose")
        );
        let mut out = Vec::new();
        assert!(run(
            &SchemaArgs {
                name: Some("info".into())
            },
            &mut out
        )?);
        let value: Value = serde_json::from_slice(&out)?;
        assert_eq!(value["type"], "object");
        assert!(!run(
            &SchemaArgs {
                name: Some("nope".into())
            },
            &mut Vec::new()
        )?);
        assert!(unknown_message("nope").contains("debug-layout"));
        Ok(())
    }
}
