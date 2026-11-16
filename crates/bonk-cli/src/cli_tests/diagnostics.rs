use super::*;

#[test]
fn diagnose_ecrit_un_rapport_json_sur_la_sortie() -> Result<(), serde_json::Error> {
    let (code, out, _) = lancer(&["diagnose"], sample());
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out)?;
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["enumeration"]["ok"], true);
    assert_eq!(
        json["enumeration"]["disks"].as_array().map(Vec::len),
        Some(3)
    );
    Ok(())
}

#[test]
fn diagnose_rapporte_un_echec_d_enumeration_avec_le_code_zero() -> Result<(), serde_json::Error> {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = execute(
        ["bonk", "diagnose"],
        &Env::default(),
        &mut out,
        &mut err,
        |_| Ok(()),
        || Err(EnumerateError::Unsupported { os: "test" }),
        |_| Ok(()),
    );
    assert_eq!(code, 0);
    let json: Value = serde_json::from_slice(&out)?;
    assert_eq!(json["enumeration"]["ok"], false);
    Ok(())
}

#[test]
fn debug_filter_explique_un_disque_propose() {
    let (code, out, err) = lancer(&["debug", "filter", "usb-cle"], sample());
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("[VERDICT  ] bus"), "{out}");
    assert!(!out.contains('§'), "{out}");
    assert!(out.contains("Verdict: offered"), "{out}");
    assert!(!out.contains("APPLIED  "), "{out}");
}

#[test]
fn debug_filter_explique_un_refus_regle_par_regle() -> Result<(), serde_json::Error> {
    let (code, out, _) = lancer(&["debug", "filter", "nvme-systeme"], sample());
    assert_eq!(code, 0);
    assert!(out.contains("[APPLIED  ] system_volume"), "{out}");
    assert!(out.contains("Verdict: forbidden"), "{out}");

    let (_, json, _) = lancer(&["debug", "filter", "nvme-systeme", "--json"], sample());
    let value: Value = serde_json::from_str(&json)?;
    assert_eq!(value["schema_version"], 2);
    assert_eq!(value["verdict"]["verdict"], "excluded");
    let rules = value["rules"].as_array().map_or(&[][..], Vec::as_slice);
    assert!(
        rules
            .iter()
            .any(|r| r["rule"] == "system_volume" && r["fired"] == true)
    );
    assert!(
        rules
            .iter()
            .any(|r| r["rule"] == "active_esp" && r["fired"] == false)
    );
    assert!(rules.iter().all(|r| r.get("cdc").is_none()), "{json}");
    Ok(())
}

#[test]
fn debug_filter_suit_les_reglages_et_refuse_un_inconnu() {
    let (_, out, _) = lancer(
        &["debug", "filter", "sata-libre", "--show-internal"],
        sample(),
    );
    assert!(out.contains("internal disks offered"), "{out}");
    assert!(out.contains("stronger confirmation"), "{out}");
    let (code, out, err) = lancer(&["debug", "filter", "absent"], sample());
    assert_eq!(code, EXIT_RUNTIME_ERROR);
    assert!(out.is_empty() && err.contains("disk not found"), "{err}");
}
