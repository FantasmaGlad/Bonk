use super::*;

#[test]
fn liste_par_defaut_ne_montre_que_les_disques_proposes() {
    let (code, out, err) = lancer(&["list"], sample());
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("usb-cle"), "{out}");
    assert!(out.contains("offered"), "{out}");
    assert!(!out.contains("nvme-systeme"), "{out}");
    assert!(!out.contains("sata-libre"), "{out}");
}

#[test]
fn all_montre_les_ecartes_avec_la_raison() {
    let (code, out, _) = lancer(&["list", "--all"], sample());
    assert_eq!(code, 0);
    assert!(
        out.contains("forbidden: system volume (/dev/nvme0n1p2)"),
        "{out}"
    );
    assert!(out.contains("/dev/nvme0n1p2 mounted on /"), "{out}");
    assert!(out.contains("hidden (internal bus"), "{out}");
}

#[test]
fn show_internal_propose_le_disque_interne_avec_avertissement() {
    let (_, out, _) = lancer(&["list", "--show-internal"], sample());
    assert!(out.contains("sata-libre"), "{out}");
    assert!(out.contains("stronger confirmation"), "{out}");
    assert!(!out.contains("nvme-systeme"), "{out}");
}

#[test]
fn aucun_disque_donne_un_message_et_le_code_zero() {
    let (code, out, _) = lancer(&["list"], vec![]);
    assert_eq!(code, 0);
    assert!(out.contains("No disk offered."), "{out}");
    assert!(out.contains("--all"), "{out}");
}

#[test]
fn colonnes_alignees() {
    let (_, out, _) = lancer(&["list", "--all"], sample());
    let columns: Vec<usize> = out
        .lines()
        .filter_map(|l| l.find("/dev/").or_else(|| l.find("PATH")))
        .collect();
    assert_eq!(columns.len(), 4, "{out}");
    assert!(
        columns.windows(2).all(|w| w[0] == w[1]),
        "{columns:?}\n{out}"
    );
}

#[test]
fn json_est_valide_et_versionne() -> Result<(), serde_json::Error> {
    let (code, out, _) = lancer(&["list", "--json", "--all"], sample());
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out)?;
    assert_eq!(json["schema_version"], 1);
    let disks = json["disks"].as_array().map_or(0, Vec::len);
    assert_eq!(disks, 3);
    assert_eq!(json["disks"][0]["id"], "nvme-systeme");
    assert_eq!(json["disks"][0]["eligibility"]["verdict"], "excluded");
    assert_eq!(json["disks"][1]["eligibility"]["verdict"], "offered");
    assert_eq!(json["disks"][2]["eligibility"]["verdict"], "hidden");
    Ok(())
}

#[test]
fn json_par_defaut_ne_contient_que_les_disques_proposes() -> Result<(), serde_json::Error> {
    let (_, out, _) = lancer(&["list", "--json"], sample());
    let json: Value = serde_json::from_str(&out)?;
    assert_eq!(json["disks"].as_array().map_or(0, Vec::len), 1);
    assert_eq!(json["disks"][0]["id"], "usb-cle");
    Ok(())
}

#[test]
fn json_sans_disque_reste_du_json_valide() -> Result<(), serde_json::Error> {
    let (code, out, _) = lancer(&["list", "--json"], vec![]);
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out)?;
    assert_eq!(json["disks"].as_array().map(Vec::len), Some(0));
    Ok(())
}

#[test]
fn echec_de_l_enumeration_donne_le_code_un_et_rien_sur_stdout() {
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
    assert_eq!(code, EXIT_RUNTIME_ERROR);
    assert!(out.is_empty());
    let err = String::from_utf8_lossy(&err);
    assert!(err.starts_with("bonk: error: "), "{err}");
}

#[test]
fn option_inconnue_donne_le_code_d_usage() {
    let (code, out, _) = lancer(&["list", "--efface-tout"], sample());
    assert_eq!(code, EXIT_USAGE);
    assert!(out.is_empty());
}

#[test]
fn disk_list_et_list_sont_equivalents() {
    let court = lancer(&["list", "--all"], sample());
    let long = lancer(&["disk", "list", "--all"], sample());
    assert_eq!(court, long);
    assert_eq!(court.0, 0);
}

#[test]
fn info_par_identifiant_ou_par_chemin() {
    for cible in ["usb-cle", "/dev/sdb"] {
        let (code, out, err) = lancer(&["info", cible], sample());
        assert_eq!((code, err.as_str()), (0, ""), "{cible}");
        assert!(out.contains("Identifier"), "{out}");
        assert!(out.contains("usb-cle"), "{out}");
        assert!(out.contains("/dev/sdb"), "{out}");
        assert!(out.contains("offered"), "{out}");
    }
    assert_eq!(
        lancer(&["disk", "info", "usb-cle"], sample()),
        lancer(&["info", "usb-cle"], sample())
    );
}

#[test]
fn info_montre_les_partitions_et_les_motifs() {
    let (code, out, _) = lancer(&["info", "nvme-systeme"], sample());
    assert_eq!(code, 0);
    assert!(out.contains("Partitions:"), "{out}");
    assert!(out.contains("/dev/nvme0n1p2"), "{out}");
    assert!(out.contains("mounted on /"), "{out}");
    assert!(out.contains("forbidden: system volume"), "{out}");
}

#[test]
fn info_suit_les_reglages_du_filtre() {
    let (_, defaut, _) = lancer(&["info", "sata-libre"], sample());
    assert!(defaut.contains("hidden"), "{defaut}");
    let (_, interne, _) = lancer(&["info", "sata-libre", "--show-internal"], sample());
    assert!(interne.contains("stronger confirmation"), "{interne}");
}

#[test]
fn info_json_est_versionne() -> Result<(), serde_json::Error> {
    let (code, out, _) = lancer(&["info", "usb-cle", "--json"], sample());
    assert_eq!(code, 0);
    let json: Value = serde_json::from_str(&out)?;
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["disk"]["id"], "usb-cle");
    assert_eq!(json["disk"]["eligibility"]["verdict"], "offered");
    Ok(())
}

#[test]
fn info_d_un_disque_inconnu_echoue_sans_rien_ecrire_sur_stdout() {
    let (code, out, err) = lancer(&["info", "n-existe-pas"], sample());
    assert_eq!(code, EXIT_RUNTIME_ERROR);
    assert!(out.is_empty());
    assert!(err.contains("disk not found: 'n-existe-pas'"), "{err}");
    assert!(err.contains("bonk list --all"), "{err}");
}

#[test]
fn info_exige_une_cible() {
    let (code, _, _) = lancer(&["info"], sample());
    assert_eq!(code, EXIT_USAGE);
}
