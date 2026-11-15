use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::*;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stored {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) values: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Origin {
    Default,
    File,
    Environment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub(crate) struct Effective {
    pub(crate) key: &'static str,
    pub(crate) value: String,
    pub(crate) origin: Origin,
    /// Name of the environment variable that imposes it, when the origin is `environment`.
    pub(crate) variable: Option<String>,
    pub(crate) help: &'static str,
}

/// `BONK_CONFIG`, else the system configuration directory.
pub(crate) fn default_path(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(explicit) = var("BONK_CONFIG").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    bonk_hal::platform::config_file(var)
}

/// A missing file gives an empty configuration.
pub(crate) fn load(path: &Path) -> Result<Stored, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map_err(|e| format!("{}: unreadable configuration file: {e}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(e) => Err(format!("{}: could not read: {e}", path.display())),
    }
}

/// In one step (temporary file, then rename): a cut never leaves a half-written file.
pub(crate) fn save(path: &Path, stored: &Stored) -> io::Result<()> {
    let fail = |e: io::Error| {
        io::Error::new(
            e.kind(),
            format!("{}: could not write: {e}", path.display()),
        )
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp).map_err(fail)?;
    let body = serde_json::to_string_pretty(&Stored {
        version: 1,
        values: stored.values.clone(),
    })
    .map_err(io::Error::other)?;
    writeln!(file, "{body}").map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&tmp, path).map_err(fail)
}

/// A valid `BONK_*` variable, else a valid file value, else the default. An invalid value is ignored, never
/// fatal for a read command.
pub(crate) fn effective(stored: &Stored, vars: &[(String, String)]) -> Vec<Effective> {
    KEYS.iter()
        .map(|key| {
            let variable = variable_name(key.name);
            let from_env = vars
                .iter()
                .find(|(name, _)| *name == variable)
                .and_then(|(_, value)| normalize(key, value).ok());
            let from_file = stored
                .values
                .get(key.name)
                .and_then(|value| normalize(key, value).ok());
            let (value, origin, variable) = match (from_env, from_file) {
                (Some(value), _) => (value, Origin::Environment, Some(variable)),
                (None, Some(value)) => (value, Origin::File, None),
                (None, None) => (key.default.to_owned(), Origin::Default, None),
            };
            Effective {
                key: key.name,
                value,
                origin,
                variable,
                help: key.help,
            }
        })
        .collect()
}

pub(crate) fn view_defaults(effective: &[Effective]) -> ViewDefaults {
    let on = |name: &str| effective.iter().any(|e| e.key == name && e.value == "true");
    ViewDefaults {
        show_internal: on("view.show_internal"),
        show_all: on("view.show_all"),
        allow_virtual: on("view.allow_virtual"),
        offline: on("network.offline"),
        lang: match value_of(effective, "lang") {
            Some("en") => Some(bonk_hal::Lang::En),
            Some("fr") => Some(bonk_hal::Lang::Fr),
            _ => None,
        },
        theme: match value_of(effective, "theme") {
            Some("light") => "light",
            Some("dark") => "dark",
            _ => "auto",
        },
    }
}

pub(crate) fn value_of<'a>(effective: &'a [Effective], key: &str) -> Option<&'a str> {
    effective
        .iter()
        .find(|e| e.key == key)
        .map(|e| e.value.as_str())
}
