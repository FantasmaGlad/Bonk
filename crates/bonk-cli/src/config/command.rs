use std::io::{self, Write};
use std::path::Path;

use clap::Subcommand;
use schemars::JsonSchema;
use serde::Serialize;

use super::*;

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigAction {
    /// List the settings with their effective value and its origin
    List {
        /// JSON output
        #[arg(long)]
        json: bool,
    },
    /// Effective value of a setting
    Get { key: String },
    /// Set a value in the configuration file
    Set { key: String, value: String },
    /// Restore a setting (or all) to its default value
    Reset { key: Option<String> },
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct ListReport {
    schema_version: u32,
    /// Configuration file used.
    file: Option<String>,
    settings: Vec<Effective>,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ListReport)).unwrap_or_default()
}

pub(super) fn invalid(message: String) -> ConfigError {
    ConfigError::Invalid(message)
}

pub(super) fn key_of(name: &str) -> Result<&'static Key, ConfigError> {
    find_key(name).ok_or_else(|| {
        let names: Vec<&str> = KEYS.iter().map(|k| k.name).collect();
        invalid(format!(
            "unknown setting: '{name}' (settings: {})",
            names.join(", ")
        ))
    })
}

pub(super) fn need_path(path: Option<&Path>) -> Result<&Path, ConfigError> {
    path.ok_or_else(|| {
        invalid("configuration directory not found (BONK_CONFIG variable?)".to_owned())
    })
}

pub(crate) fn run(
    action: &ConfigAction,
    path: Option<&Path>,
    vars: &[(String, String)],
    out: &mut dyn Write,
) -> Result<(), ConfigError> {
    let mut stored = match path {
        Some(p) => load(p).map_err(invalid)?,
        None => Stored::default(),
    };
    match action {
        ConfigAction::List { json } => list(effective(&stored, vars), path, *json, out),
        ConfigAction::Get { key } => {
            let key = key_of(key)?;
            let settings = effective(&stored, vars);
            writeln!(
                out,
                "{}",
                value_of(&settings, key.name).unwrap_or(key.default)
            )?;
            Ok(())
        }
        ConfigAction::Set { key, value } => {
            let key = key_of(key)?;
            let value = normalize(key, value).map_err(invalid)?;
            stored.values.insert(key.name.to_owned(), value.clone());
            save(need_path(path)?, &stored)?;
            writeln!(out, "{} = {value}", key.name)?;
            Ok(())
        }
        ConfigAction::Reset { key } => {
            match key {
                Some(name) => {
                    stored.values.remove(key_of(name)?.name);
                }
                None => stored.values.clear(),
            }
            save(need_path(path)?, &stored)?;
            writeln!(out, "Settings restored to their default values.")?;
            Ok(())
        }
    }
}

pub(super) fn list(
    settings: Vec<Effective>,
    path: Option<&Path>,
    json: bool,
    out: &mut dyn Write,
) -> Result<(), ConfigError> {
    if json {
        let report = ListReport {
            schema_version: 1,
            file: path.map(|p| p.display().to_string()),
            settings,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)?;
        return Ok(());
    }
    let width = settings.iter().map(|e| e.key.len()).max().unwrap_or(0);
    for e in &settings {
        let origin = match (e.origin, &e.variable) {
            (Origin::Default, _) => "default".to_owned(),
            (Origin::File, _) => "file".to_owned(),
            (Origin::Environment, Some(v)) => format!("variable {v}"),
            (Origin::Environment, None) => "variable".to_owned(),
        };
        writeln!(out, "{:<width$}  {}  ({origin})", e.key, e.value)?;
    }
    Ok(())
}

pub(crate) enum ConfigError {
    Invalid(String),
    Output(io::Error),
}

impl From<io::Error> for ConfigError {
    fn from(error: io::Error) -> Self {
        Self::Output(error)
    }
}
