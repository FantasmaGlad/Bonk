use std::fs;
use std::path::{Path, PathBuf};

use super::*;

struct TempFile(PathBuf);
impl TempFile {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bonk-config-{}-{label}", std::process::id()));
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

fn texte(action: &ConfigAction, path: &Path, vars: &[(String, String)]) -> Result<String, String> {
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
    assert!(erreur.is_some_and(|e| e.contains("unknown setting") && e.contains("view.show_all")));
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
