//! Réglages persistants : `bonk config list|get|set|reset`.
//!
//! Priorité : option de la ligne de commande, puis variable `BONK_*`, puis fichier, puis valeur par
//! défaut. `config list` montre la valeur effective et son origine, pour comprendre d'où vient un
//! comportement. Le fichier est un petit JSON de chaînes, validées clé par clé à l'écriture.

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

/// Les réglages existants. Les langues ne comptent que celles qui sont réellement livrées.
pub(crate) const KEYS: &[Key] = &[
    Key {
        name: "lang",
        kind: Kind::Choice(&["fr"]),
        default: "fr",
        help: "langue de l'interface et des sorties lisibles (l'anglais n'est pas encore livré)",
    },
    Key {
        name: "log.format",
        kind: Kind::Choice(&["text", "json"]),
        default: "text",
        help: "format du journal quand --log-format n'est pas donné",
    },
    Key {
        name: "view.show_internal",
        kind: Kind::Bool,
        default: "false",
        help: "proposer aussi les disques internes non système (fenêtre, list, info)",
    },
    Key {
        name: "view.show_all",
        kind: Kind::Bool,
        default: "false",
        help: "afficher aussi les disques écartés (fenêtre, list)",
    },
    Key {
        name: "view.allow_virtual",
        kind: Kind::Bool,
        default: "false",
        help: "mode test : autoriser les disques virtuels (fenêtre, list, info)",
    },
];

/// Contenu du fichier de configuration.
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
    /// Nom de la variable d'environnement qui l'impose, quand l'origine est `environment`.
    pub(crate) variable: Option<String>,
    pub(crate) help: &'static str,
}

/// Réglages d'affichage qui s'appliquent à la fenêtre et aux commandes de lecture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewDefaults {
    pub show_internal: bool,
    pub show_all: bool,
    pub allow_virtual: bool,
}

/// Nom de la variable d'environnement d'une clé : `view.show_all` donne `BONK_VIEW_SHOW_ALL`.
pub(crate) fn variable_name(key: &str) -> String {
    format!("BONK_{}", key.replace('.', "_").to_uppercase())
}

pub(crate) fn find_key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.name == name)
}

/// Valide et normalise une valeur pour une clé : `oui`, `1`, `true` donnent `true`.
pub(crate) fn normalize(key: &Key, value: &str) -> Result<String, String> {
    let value = value.trim();
    match &key.kind {
        Kind::Bool => match value.to_lowercase().as_str() {
            "true" | "1" | "oui" | "yes" | "on" => Ok("true".to_owned()),
            "false" | "0" | "non" | "no" | "off" => Ok("false".to_owned()),
            _ => Err(format!(
                "« {value} » n'est pas un booléen pour {} (true ou false)",
                key.name
            )),
        },
        Kind::Choice(choices) => {
            let lower = value.to_lowercase();
            if choices.contains(&lower.as_str()) {
                Ok(lower)
            } else {
                Err(format!(
                    "« {value} » n'est pas une valeur de {} (valeurs : {})",
                    key.name,
                    choices.join(", ")
                ))
            }
        }
    }
}

/// Emplacement du fichier : `BONK_CONFIG`, sinon le dossier de configuration du système.
pub(crate) fn default_path(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    if let Some(explicit) = var("BONK_CONFIG").filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(explicit));
    }
    bonk_hal::platform::config_file(var)
}

/// Lit le fichier ; un fichier absent donne une configuration vide.
pub(crate) fn load(path: &Path) -> Result<Stored, String> {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).map_err(|e| {
            format!(
                "{} : fichier de configuration illisible : {e}",
                path.display()
            )
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Stored::default()),
        Err(e) => Err(format!("{} : lecture impossible : {e}", path.display())),
    }
}

/// Écrit le fichier d'un seul geste (fichier temporaire puis renommage) : une coupure ne laisse jamais
/// un fichier à moitié écrit.
pub(crate) fn save(path: &Path, stored: &Stored) -> Result<(), String> {
    let fail = |e: io::Error| format!("{} : écriture impossible : {e}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp).map_err(fail)?;
    let body = serde_json::to_string_pretty(&Stored {
        version: 1,
        values: stored.values.clone(),
    })
    .map_err(|e| e.to_string())?;
    writeln!(file, "{body}").map_err(fail)?;
    file.sync_all().map_err(fail)?;
    fs::rename(&tmp, path).map_err(fail)
}

/// Valeurs effectives : variable `BONK_*` valide, sinon fichier valide, sinon défaut. Une valeur
/// invalide dans l'environnement ou le fichier est ignorée, jamais fatale pour une commande de lecture.
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
    /// Liste les réglages avec leur valeur effective et son origine
    List {
        /// Sortie JSON
        #[arg(long)]
        json: bool,
    },
    /// Valeur effective d'un réglage
    Get { key: String },
    /// Fixe un réglage dans le fichier de configuration
    Set { key: String, value: String },
    /// Remet un réglage (ou tous) à sa valeur par défaut
    Reset { key: Option<String> },
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct ListReport {
    schema_version: u32,
    /// Fichier de configuration utilisé.
    file: Option<String>,
    settings: Vec<Effective>,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ListReport)).unwrap_or_default()
}

/// Résultat d'une commande `config` : texte d'erreur à afficher, ou rien.
pub(crate) fn run(
    action: &ConfigAction,
    path: Option<&Path>,
    vars: &[(String, String)],
    out: &mut dyn Write,
) -> Result<(), ConfigError> {
    let invalid = |message: String| ConfigError::Invalid(message);
    let key_of = |name: &str| {
        find_key(name).ok_or_else(|| {
            let names: Vec<&str> = KEYS.iter().map(|k| k.name).collect();
            invalid(format!(
                "réglage inconnu : « {name} » (réglages : {})",
                names.join(", ")
            ))
        })
    };
    let need_path = || {
        path.ok_or_else(|| {
            invalid("dossier de configuration introuvable (variable BONK_CONFIG ?)".to_owned())
        })
    };
    let stored = match path {
        Some(p) => load(p).map_err(invalid)?,
        None => Stored::default(),
    };
    match action {
        ConfigAction::List { json } => {
            let settings = effective(&stored, vars);
            if *json {
                let report = ListReport {
                    schema_version: 1,
                    file: path.map(|p| p.display().to_string()),
                    settings,
                };
                serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
                writeln!(out)?;
            } else {
                let width = settings.iter().map(|e| e.key.len()).max().unwrap_or(0);
                for e in &settings {
                    let origin = match (e.origin, &e.variable) {
                        (Origin::Default, _) => "défaut".to_owned(),
                        (Origin::File, _) => "fichier".to_owned(),
                        (Origin::Environment, Some(v)) => format!("variable {v}"),
                        (Origin::Environment, None) => "variable".to_owned(),
                    };
                    writeln!(out, "{:<width$}  {}  ({origin})", e.key, e.value)?;
                }
            }
        }
        ConfigAction::Get { key } => {
            let key = key_of(key)?;
            let settings = effective(&stored, vars);
            writeln!(
                out,
                "{}",
                value_of(&settings, key.name).unwrap_or(key.default)
            )?;
        }
        ConfigAction::Set { key, value } => {
            let key = key_of(key)?;
            let value = normalize(key, value).map_err(invalid)?;
            let mut stored = stored;
            stored.values.insert(key.name.to_owned(), value.clone());
            save(need_path()?, &stored).map_err(invalid)?;
            writeln!(out, "{} = {value}", key.name)?;
        }
        ConfigAction::Reset { key } => {
            let mut stored = stored;
            match key {
                Some(name) => {
                    let key = key_of(name)?;
                    stored.values.remove(key.name);
                }
                None => stored.values.clear(),
            }
            save(need_path()?, &stored).map_err(invalid)?;
            writeln!(out, "Réglages remis à leur valeur par défaut.")?;
        }
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
        let erreur = normalize(lang, "en").err().unwrap_or_default();
        assert!(erreur.contains("valeurs : fr"), "{erreur}");
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
                allow_virtual: false
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
        // Absent : tout est par défaut, rien n'est créé par une lecture.
        let list = texte(&ConfigAction::List { json: false }, path, &[])?;
        assert!(
            list.contains("view.show_all") && list.contains("(défaut)"),
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
            list.contains("view.show_all") && list.contains("(fichier)"),
            "{list}"
        );

        // La variable d'environnement l'emporte, et la liste dit laquelle.
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
            erreur.is_some_and(|e| e.contains("réglage inconnu") && e.contains("view.show_all"))
        );
        let erreur = texte(
            &ConfigAction::Set {
                key: "lang".into(),
                value: "en".into(),
            },
            &file.0,
            &[],
        )
        .err();
        assert!(erreur.is_some_and(|e| e.contains("valeurs : fr")));
        assert!(!file.0.exists());
        Ok(())
    }

    #[test]
    fn fichier_corrompu_signale_et_non_ecrase() -> Result<(), Box<dyn std::error::Error>> {
        let file = TempFile::new("corrompu");
        fs::create_dir_all(file.0.parent().ok_or("parent")?)?;
        fs::write(&file.0, "{ pas du json")?;
        let erreur = texte(&ConfigAction::List { json: false }, &file.0, &[]).err();
        assert!(erreur.is_some_and(|e| e.contains("fichier de configuration illisible")));
        // `set` ne remplace pas en silence un fichier qu'il ne comprend pas.
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
