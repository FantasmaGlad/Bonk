use super::*;

#[test]
fn decoupage_des_mots_comme_un_shell() {
    assert_eq!(
        split_words("bonk info 'clé usb' --json").ok(),
        Some(vec![
            "bonk".into(),
            "info".into(),
            "clé usb".into(),
            "--json".into()
        ])
    );
    assert_eq!(
        split_words("bonk info 'it'\\''s'").ok(),
        Some(vec!["bonk".into(), "info".into(), "it's".into()])
    );
    assert_eq!(
        split_words("a  ''  b").ok(),
        Some(vec!["a".into(), String::new(), "b".into()])
    );
    assert!(split_words("bonk info 'ouvert").is_err());
}

#[test]
fn une_ligne_valide_est_acceptee_et_une_invalide_refusee() {
    assert!(check_command_line("bonk list --all --show-internal").is_ok());
    assert!(check_command_line("bonk disk info usb-cle").is_ok());
    assert!(check_command_line("bonk efface --tout").is_err());
    assert!(check_command_line("bonk info").is_err());
}

#[test]
fn toute_erreur_porte_son_code_en_fin_de_message() {
    let (_, _, err) = lancer(&["info", "absent"], sample());
    assert!(err.trim_end().ends_with("[E1003]"), "{err}");
    let r = lancer_avec(&["list"], Some("gui=bavard"), sample(), Ok(()));
    assert!(r.err.trim_end().ends_with("[E5001]"), "{}", r.err);
    let r = lancer_avec(&[], None, vec![], Err("x".to_owned()));
    assert!(r.err.trim_end().ends_with("[E5003]"), "{}", r.err);
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = execute(
        ["bonk", "list"],
        &Env::default(),
        &mut out,
        &mut err,
        |_| Ok(()),
        || Err(EnumerateError::Unsupported { os: "test" }),
        |_| Ok(()),
    );
    assert_eq!(code, 1);
    assert!(
        String::from_utf8_lossy(&err)
            .trim_end()
            .ends_with("[E1002]")
    );
}

#[test]
fn explain_donne_le_sens_d_un_code_et_refuse_un_code_inconnu() {
    let (code, out, _) = lancer(&["explain", "E1003"], vec![]);
    assert_eq!(code, 0);
    assert!(out.starts_with("E1003: Disk not found"), "{out}");
    let (code, out, _) = lancer(&["explain"], vec![]);
    assert_eq!(code, 0);
    assert!(out.lines().count() >= 20, "{out}");
    let (code, out, err) = lancer(&["explain", "E9999"], vec![]);
    assert_eq!(code, EXIT_USAGE);
    assert!(out.is_empty() && err.contains("unknown code"), "{err}");
    assert!(err.trim_end().ends_with("[E0001]"), "{err}");
}

#[test]
fn schema_liste_et_decrit() -> Result<(), serde_json::Error> {
    let (code, out, _) = lancer(&["schema"], vec![]);
    assert_eq!(code, 0);
    assert!(out.lines().any(|l| l == "list") && out.lines().any(|l| l == "debug-layout"));
    let (code, out, _) = lancer(&["schema", "list"], vec![]);
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out)?;
    assert_eq!(json["type"], "object");
    let (code, _, err) = lancer(&["schema", "inconnu"], vec![]);
    assert_eq!(code, EXIT_USAGE);
    assert!(
        err.contains("schemas:") && err.trim_end().ends_with("[E0001]"),
        "{err}"
    );
    Ok(())
}

/// The real `list --json` output follows its own schema: same top-level fields.
#[test]
fn la_sortie_json_correspond_a_son_schema() -> Result<(), serde_json::Error> {
    let (_, out, _) = lancer(&["list", "--json", "--all"], sample());
    let real: Value = serde_json::from_str(&out)?;
    let (_, schema, _) = lancer(&["schema", "list"], vec![]);
    let schema: Value = serde_json::from_str(&schema)?;
    let props = schema["properties"]
        .as_object()
        .map(|p| p.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    let mut keys: Vec<String> = real
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    let mut props = props;
    props.sort();
    assert_eq!(keys, props);
    Ok(())
}

#[test]
fn completions_pour_chaque_shell() {
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (code, out, err) = lancer(&["completions", shell], vec![]);
        assert_eq!(code, 0, "{shell} : {err}");
        assert!(out.contains("bonk") && out.contains("diagnose"), "{shell}");
    }
    let (code, _, _) = lancer(&["completions", "tcsh"], vec![]);
    assert_eq!(code, EXIT_USAGE);
}
