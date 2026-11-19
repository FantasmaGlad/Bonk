use super::*;

fn lancer_config(
    args: &[&str],
    file: &std::path::Path,
    vars: &[(&str, &str)],
    disks: Vec<Disk>,
) -> Resultat {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let vues = RefCell::new(None);
    let env = Env {
        bonk_log: None,
        config_path: Some(file.to_path_buf()),
        vars: vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect(),
        io: job::Io::default(),
        catalog: crate::catalog::Access::default(),
        privileged: bonk_hal::platform::current().is_privileged(),
    };
    let code = execute(
        std::iter::once("bonk").chain(args.iter().copied()),
        &env,
        &mut out,
        &mut err,
        |_| Ok(()),
        || Ok(disks.clone()),
        |view| {
            *vues.borrow_mut() = Some(view);
            Ok(())
        },
    );
    Resultat {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
        journal_installe: false,
        fenetre_ouverte: vues.borrow().is_some(),
    }
}

fn fichier_de_config(label: &str) -> std::path::PathBuf {
    std::env::temp_dir()
        .join(format!("bonk-cli-config-{}-{label}", std::process::id()))
        .join("config.json")
}

/// What the window opens with when `bonk` starts without arguments on this configuration file.
fn vue_de_la_fenetre(file: &std::path::Path) -> ViewDefaults {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let vue = RefCell::new(ViewDefaults::default());
    let env = Env {
        config_path: Some(file.to_path_buf()),
        ..Env::default()
    };
    let code = execute(
        ["bonk"],
        &env,
        &mut out,
        &mut err,
        |_| Ok(()),
        || Ok(vec![]),
        |v| {
            *vue.borrow_mut() = v.view;
            Ok(())
        },
    );
    assert_eq!(code, 0);
    vue.into_inner()
}

#[test]
fn config_set_puis_list_puis_effet_sur_list_et_la_fenetre() -> Result<(), Box<dyn std::error::Error>>
{
    let file = fichier_de_config("effet");
    let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
    let r = lancer_config(
        &["config", "set", "view.show_all", "true"],
        &file,
        &[],
        vec![],
    );
    assert_eq!((r.code, r.err.as_str()), (0, ""), "{}", r.err);
    let r = lancer_config(&["config", "list"], &file, &[], vec![]);
    assert!(
        r.out.contains("view.show_all") && r.out.contains("(file)"),
        "{}",
        r.out
    );
    // Without an option, `list` follows the setting and also shows excluded disks.
    let r = lancer_config(&["list"], &file, &[], sample());
    assert!(r.out.contains("nvme-systeme"), "{}", r.out);
    // The environment variable wins over the file.
    let r = lancer_config(
        &["list"],
        &file,
        &[("BONK_VIEW_SHOW_ALL", "false")],
        sample(),
    );
    assert!(!r.out.contains("nvme-systeme"), "{}", r.out);
    let vue = vue_de_la_fenetre(&file);
    assert!(vue.show_all && !vue.show_internal);
    let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
    Ok(())
}

#[test]
fn config_refuse_une_valeur_invalide_avec_le_code_d_usage() {
    let file = fichier_de_config("refus");
    let r = lancer_config(&["config", "set", "lang", "de"], &file, &[], vec![]);
    assert_eq!(r.code, EXIT_USAGE);
    assert!(
        r.err.contains("values: auto, en, fr") && r.err.trim_end().ends_with("[E5002]"),
        "{}",
        r.err
    );
    assert!(!file.exists());
    let r = lancer_config(&["config", "get", "inconnu"], &file, &[], vec![]);
    assert_eq!(r.code, EXIT_USAGE);
}

#[test]
fn un_fichier_de_config_illisible_n_empeche_pas_list() -> Result<(), Box<dyn std::error::Error>> {
    let file = fichier_de_config("casse");
    std::fs::create_dir_all(file.parent().ok_or("parent")?)?;
    std::fs::write(&file, "{ casse")?;
    let r = lancer_config(&["list"], &file, &[], sample());
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.out.contains("usb-cle"));
    let r = lancer_config(&["config", "list"], &file, &[], vec![]);
    assert_eq!(r.code, EXIT_USAGE);
    assert!(r.err.contains("unreadable"), "{}", r.err);
    let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
    Ok(())
}

/// A file that cannot be written is not an invalid configuration: the setting was valid.
#[test]
fn config_non_enregistrable_est_une_erreur_d_ecriture() -> Result<(), Box<dyn std::error::Error>> {
    let file = fichier_de_config("ecriture");
    let dossier = file.parent().ok_or("parent")?;
    std::fs::create_dir_all(dossier)?;
    // A folder where the temporary file goes: the write fails whatever the user's rights.
    std::fs::create_dir_all(file.with_extension("json.tmp"))?;
    let r = lancer_config(&["config", "set", "lang", "fr"], &file, &[], vec![]);
    assert_eq!(r.code, crate::errors::EXIT_RUNTIME_ERROR, "{}", r.err);
    assert!(r.err.trim_end().ends_with("[E5004]"), "{}", r.err);
    std::fs::remove_dir_all(dossier)?;
    Ok(())
}
