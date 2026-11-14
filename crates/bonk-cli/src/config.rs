use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use clap::Subcommand;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub(crate) enum Kind {
    Bool,
    Choice(&'static [&'static str]),
}

pub(crate) struct Key {
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    pub(crate) default: &'static str,
    pub(crate) help: &'static str,
}

pub(crate) const KEYS: &[Key] = &[
    Key {
        name: "lang",
        kind: Kind::Choice(&["auto", "en", "fr"]),
        default: "auto",
        help: "window language; auto follows the system; the command line is always in English",
    },
    Key {
        name: "theme",
        kind: Kind::Choice(&["auto", "light", "dark"]),
        default: "auto",
        help: "window theme; auto follows the system at each start",
    },
    Key {
        name: "log.format",
        kind: Kind::Choice(&["text", "json"]),
        default: "text",
        help: "log format when --log-format is not given",
    },
    Key {
        name: "view.show_internal",
        kind: Kind::Bool,
        default: "false",
        help: "also offer internal non-system disks (window, list, info)",
    },
    Key {
        name: "view.show_all",
        kind: Kind::Bool,
        default: "false",
        help: "also show excluded disks (window, list)",
    },
    Key {
        name: "view.allow_virtual",
        kind: Kind::Bool,
        default: "false",
        help: "test mode: allow virtual disks (window, list, info)",
    },
    Key {
        name: "network.offline",
        kind: Kind::Bool,
        default: "false",
        help: "window: never read the catalog nor download; local image files only",
    },
];

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

/// Display settings shared by the window and read commands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewDefaults {
    pub show_internal: bool,
    pub show_all: bool,
    pub allow_virtual: bool,
    /// The window reaches no network: no catalog, no download.
    pub offline: bool,
    /// `None`: the system's.
    pub lang: Option<bonk_hal::Lang>,
    /// As stored: `auto`, `light` or `dark`; the window reads it.
    pub theme: &'static str,
}

impl ViewDefaults {
    /// To list or verify, the settings widen what was asked; writing uses what was asked alone.
    #[must_use]
    pub fn widen(&self, asked: bonk_hal::FilterOptions) -> bonk_hal::FilterOptions {
        bonk_hal::FilterOptions {
            show_internal: asked.show_internal || self.show_internal,
            allow_virtual: asked.allow_virtual || self.allow_virtual,
        }
    }
}

/// What the window opens on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowRequest {
    pub view: ViewDefaults,
    /// Catalog image to choose at once (`gui --image`, `gui --link`).
    pub image: Option<String>,
}

/// `view.show_all` gives `BONK_VIEW_SHOW_ALL`.
pub(crate) fn variable_name(key: &str) -> String {
    format!("BONK_{}", key.replace('.', "_").to_uppercase())
}

pub(crate) fn find_key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.name == name)
}

/// `oui`, `1`, `true` give `true`.
pub(crate) fn normalize(key: &Key, value: &str) -> Result<String, String> {
    let value = value.trim();
    match &key.kind {
        Kind::Bool => match value.to_lowercase().as_str() {
            "true" | "1" | "oui" | "yes" | "on" => Ok("true".to_owned()),
            "false" | "0" | "non" | "no" | "off" => Ok("false".to_owned()),
            _ => Err(format!(
                "'{value}' is not a boolean for {} (true or false)",
                key.name
            )),
        },
        Kind::Choice(choices) => {
            let lower = value.to_lowercase();
            if choices.contains(&lower.as_str()) {
                Ok(lower)
            } else {
                Err(format!(
                    "'{value}' is not a value of {} (values: {})",
                    key.name,
                    choices.join(", ")
                ))
            }
        }
    }
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

fn invalid(message: String) -> ConfigError {
    ConfigError::Invalid(message)
}

fn key_of(name: &str) -> Result<&'static Key, ConfigError> {
    find_key(name).ok_or_else(|| {
        let names: Vec<&str> = KEYS.iter().map(|k| k.name).collect();
        invalid(format!(
            "unknown setting: '{name}' (settings: {})",
            names.join(", ")
        ))
    })
}

fn need_path(path: Option<&Path>) -> Result<&Path, ConfigError> {
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

fn list(
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

#[cfg(test)]
mod tests {
    use super::*;

    struct TempFile(PathBuf);
    impl TempFile {
        fn new(label: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("bonk-config-{}-{label}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Self(dir.join("sous-dossier").join("config.json"))
        }
    }
    impl Drop for TempFile {
        fn drop(&mut self) {
            if let Some(dir) = self.0.parent().and_then(Path::parent) {
                let _ = fs::remove_dir_all(dir);
            }
        }
    }

    fn texte(
        action: &ConfigAction,
        path: &Path,
        vars: &[(String, String)],
    ) -> Result<String, String> {
        let mut out = Vec::new();
        match run(action, Some(path), vars, &mut out) {
            Ok(()) => Ok(String::from_utf8_lossy(&out).into_owned()),
            Err(ConfigError::Invalid(m)) => Err(m),
            Err(ConfigError::Output(e)) => Err(e.to_string()),
        }
    }

    #[test]
    fn variables_d_environnement() {
        assert_eq!(variable_name("view.show_all"), "BONK_VIEW_SHOW_ALL");
        assert_eq!(variable_name("lang"), "BONK_LANG");
        assert_eq!(variable_name("log.format"), "BONK_LOG_FORMAT");
    }

    #[test]
    fn normalisation_des_valeurs() -> Result<(), String> {
        let bool_key = find_key("view.show_all").ok_or("clé")?;
        for yes in ["true", "TRUE", "1", "oui", " yes "] {
            assert_eq!(normalize(bool_key, yes)?, "true");
        }
        for no in ["false", "0", "non", "off"] {
            assert_eq!(normalize(bool_key, no)?, "false");
        }
        assert!(normalize(bool_key, "peut-être").is_err());
        let lang = find_key("lang").ok_or("clé")?;
        assert_eq!(normalize(lang, "FR")?, "fr");
        assert_eq!(normalize(lang, "en")?, "en");
        let erreur = normalize(lang, "de").err().unwrap_or_default();
        assert!(erreur.contains("values: auto, en, fr"), "{erreur}");
        Ok(())
    }

    #[test]
    fn priorite_variable_fichier_defaut() {
        let mut stored = Stored::default();
        stored.values.insert("view.show_all".into(), "true".into());
        stored
            .values
            .insert("view.show_internal".into(), "true".into());
        let vars = vec![("BONK_VIEW_SHOW_INTERNAL".to_owned(), "non".to_owned())];
        let eff = effective(&stored, &vars);
        let get = |k: &str| eff.iter().find(|e| e.key == k).cloned();
        let all = get("view.show_all");
        assert!(all.is_some_and(|e| e.value == "true" && e.origin == Origin::File));
        let internal = get("view.show_internal");
        assert!(internal.is_some_and(|e| e.value == "false"
            && e.origin == Origin::Environment
            && e.variable.as_deref() == Some("BONK_VIEW_SHOW_INTERNAL")));
        let virt = get("view.allow_virtual");
        assert!(virt.is_some_and(|e| e.value == "false" && e.origin == Origin::Default));
        assert_eq!(
            view_defaults(&eff),
            ViewDefaults {
                show_internal: false,
                show_all: true,
                allow_virtual: false,
                offline: false,
                lang: None,
                theme: "auto",
            }
        );
    }

    #[test]
    fn une_valeur_invalide_dans_le_fichier_ou_l_environnement_est_ignoree() {
        let mut stored = Stored::default();
        stored.values.insert("view.show_all".into(), "bof".into());
        let vars = vec![("BONK_LANG".to_owned(), "klingon".to_owned())];
        let eff = effective(&stored, &vars);
        assert!(eff.iter().all(|e| e.origin == Origin::Default));
    }

    #[test]
    fn set_get_list_reset_de_bout_en_bout() -> Result<(), String> {
        let file = TempFile::new("cycle");
        let path = &file.0;
        // Missing: all defaults, nothing created by a read.
        let list = texte(&ConfigAction::List { json: false }, path, &[])?;
        assert!(
            list.contains("view.show_all") && list.contains("(default)"),
            "{list}"
        );
        assert!(!path.exists());

        texte(
            &ConfigAction::Set {
                key: "view.show_all".into(),
                value: "oui".into(),
            },
            path,
            &[],
        )?;
        assert!(path.exists());
        assert_eq!(
            texte(
                &ConfigAction::Get {
                    key: "view.show_all".into()
                },
                path,
                &[]
            )?,
            "true\n"
        );
        let list = texte(&ConfigAction::List { json: false }, path, &[])?;
        assert!(
            list.contains("view.show_all") && list.contains("(file)"),
            "{list}"
        );

        // The environment variable wins, and the list says which.
        let vars = [("BONK_VIEW_SHOW_ALL".to_owned(), "false".to_owned())];
        assert_eq!(
            texte(
                &ConfigAction::Get {
                    key: "view.show_all".into()
                },
                path,
                &vars
            )?,
            "false\n"
        );
        assert!(
            texte(&ConfigAction::List { json: false }, path, &vars)?
                .contains("variable BONK_VIEW_SHOW_ALL")
        );

        texte(
            &ConfigAction::Reset {
                key: Some("view.show_all".into()),
            },
            path,
            &[],
        )?;
        assert_eq!(
            texte(
                &ConfigAction::Get {
                    key: "view.show_all".into()
                },
                path,
                &[]
            )?,
            "false\n"
        );
        texte(
            &ConfigAction::Set {
                key: "log.format".into(),
                value: "json".into(),
            },
            path,
            &[],
        )?;
        texte(&ConfigAction::Reset { key: None }, path, &[])?;
        assert!(load(path)?.values.is_empty());
        Ok(())
    }

    #[test]
    fn set_refuse_cle_inconnue_et_valeur_invalide_sans_toucher_au_fichier() -> Result<(), String> {
        let file = TempFile::new("refus");
        let erreur = texte(
            &ConfigAction::Set {
                key: "n.importe.quoi".into(),
                value: "1".into(),
            },
            &file.0,
            &[],
        )
        .err();
        assert!(
            erreur.is_some_and(|e| e.contains("unknown setting") && e.contains("view.show_all"))
        );
        let erreur = texte(
            &ConfigAction::Set {
                key: "lang".into(),
                value: "de".into(),
            },
            &file.0,
            &[],
        )
        .err();
        assert!(erreur.is_some_and(|e| e.contains("values: auto, en, fr")));
        assert!(!file.0.exists());
        Ok(())
    }

    #[test]
    fn fichier_corrompu_signale_et_non_ecrase() -> Result<(), Box<dyn std::error::Error>> {
        let file = TempFile::new("corrompu");
        fs::create_dir_all(file.0.parent().ok_or("parent")?)?;
        fs::write(&file.0, "{ pas du json")?;
        let erreur = texte(&ConfigAction::List { json: false }, &file.0, &[]).err();
        assert!(erreur.is_some_and(|e| e.contains("unreadable configuration file")));
        // `set` does not silently replace a file it does not understand.
        assert!(
            texte(
                &ConfigAction::Set {
                    key: "lang".into(),
                    value: "fr".into()
                },
                &file.0,
                &[]
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&file.0)?, "{ pas du json");
        Ok(())
    }

    #[test]
    fn json_de_la_liste_versionne() -> Result<(), Box<dyn std::error::Error>> {
        let file = TempFile::new("json");
        let out = texte(&ConfigAction::List { json: true }, &file.0, &[])?;
        let value: serde_json::Value = serde_json::from_str(&out)?;
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["settings"].as_array().map(Vec::len), Some(KEYS.len()));
        assert_eq!(value["settings"][0]["origin"], "default");
        Ok(())
    }

    #[test]
    fn emplacement_par_defaut() {
        let var = |m: &'static [(&'static str, &'static str)]| {
            move |n: &str| {
                m.iter()
                    .find(|(k, _)| *k == n)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        assert_eq!(
            default_path(&var(&[("BONK_CONFIG", "/x/c.json")])),
            Some(PathBuf::from("/x/c.json"))
        );
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            assert_eq!(
                default_path(&var(&[("XDG_CONFIG_HOME", "/cfg")])),
                Some(PathBuf::from("/cfg/bonk/config.json"))
            );
            assert_eq!(
                default_path(&var(&[("HOME", "/home/a")])),
                Some(PathBuf::from("/home/a/.config/bonk/config.json"))
            );
            assert_eq!(default_path(&var(&[])), None);
        }
    }
}
