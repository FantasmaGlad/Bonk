use std::io::{self, Write};

use clap::Args;
use serde_json::Value;

use crate::{
    cache, catalog, config, debug, diagnose, errors, format, info, list, serve, transfer, vault,
};

/// Command name and generator of its output schema.
type Entry = (&'static str, fn() -> Value);

pub(crate) fn registry() -> Vec<Entry> {
    vec![
        ("list", list::schema),
        ("info", info::schema),
        ("diagnose", diagnose::schema),
        ("explain", errors::schema),
        ("config", config::schema),
        ("cache", cache::schema),
        ("catalog", catalog::schema),
        ("debug-filter", debug::schema_filter),
        ("debug-sources", debug::schema_sources),
        ("debug-layout", debug::schema_layout),
        ("write", transfer::schema_write),
        ("write-extract", transfer::schema_write_extract),
        ("write-plan", transfer::schema_plan),
        ("verify", transfer::schema_verify),
        ("repair-gpt", transfer::schema_repair),
        ("progress", transfer::schema_progress),
        ("progress-error", transfer::schema_error),
        ("format", format::schema_format),
        ("capabilities", format::schema_capabilities),
        ("vault-init", vault::schema_init),
        ("vault-list", vault::schema_list),
        ("vault-add", vault::schema_add),
        ("vault-remove", vault::schema_remove),
        ("vault-update", vault::schema_update),
        ("serve-request", serve::schema_request),
        ("serve-response", serve::schema_response),
    ]
}

/// For the service; `None` for an unknown name.
pub(crate) fn value(name: &str) -> Option<Value> {
    registry()
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, make)| make())
}

pub(crate) fn names() -> Vec<&'static str> {
    registry().iter().map(|(n, _)| *n).collect()
}

/// JSON Schema of a command's output
#[derive(Debug, Args)]
// Without it clap would reuse the doc comment above as long help.
#[command(long_about = None)]
pub(crate) struct SchemaArgs {
    /// Command whose schema is wanted; without a name, list the commands
    name: Option<String>,
}

impl SchemaArgs {
    pub(crate) fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// Or the list. Returns `false` for an unknown name.
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
    format!("unknown schema: '{name}' (schemas: {})", names.join(", "))
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
            // Service envelopes are versioned by the protocol (`ready` event), not by a field: only command
            // outputs carry `schema_version`.
            let key = if name.starts_with("serve-") {
                "id"
            } else {
                "schema_version"
            };
            assert!(
                properties.is_some_and(|p| p.contains_key(key)),
                "{name} : pas de {key}"
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
