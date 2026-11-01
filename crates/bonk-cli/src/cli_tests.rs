use std::cell::RefCell;

use bonk_hal::{Bus, DiskKind, Partition};
use serde_json::Value;

use super::*;
use crate::errors::EXIT_RUNTIME_ERROR;

fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
    Partition {
        path: path.to_owned(),
        label: None,
        fs_type: Some("ext4".to_owned()),
        size_bytes: 1 << 30,
        mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
        holders: vec![],
        is_system_volume: system,
        is_active_esp: false,
    }
}

fn disk(id: &str, path: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
    Disk {
        id: id.to_owned(),
        path: path.to_owned(),
        model: "Modèle de test".to_owned(),
        serial: None,
        size_bytes: 16_008_609_792,
        bus,
        kind: DiskKind::Physical,
        removable_flag: false,
        fs_type: None,
        holders: vec![],
        partitions,
    }
}

fn sample() -> Vec<Disk> {
    vec![
        disk(
            "nvme-systeme",
            "/dev/nvme0n1",
            Bus::Nvme,
            vec![partition("/dev/nvme0n1p2", &["/"], true)],
        ),
        disk("usb-cle", "/dev/sdb", Bus::Usb, vec![]),
        disk("sata-libre", "/dev/sda", Bus::Sata, vec![]),
    ]
}

/// Ce que `execute` a fait : code, sorties, et réglages de journal reçus.
struct Resultat {
    code: u8,
    out: String,
    err: String,
    journal_installe: bool,
    fenetre_ouverte: bool,
}

fn lancer_avec(
    args: &[&str],
    env_log: Option<&str>,
    disks: Vec<Disk>,
    fenetre: Result<(), String>,
) -> Resultat {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let journal = RefCell::new(false);
    let ouverte = RefCell::new(false);
    let code = execute(
        std::iter::once("bonk").chain(args.iter().copied()),
        &Env {
            bonk_log: env_log.map(str::to_owned),
            ..Env::default()
        },
        &mut out,
        &mut err,
        |_| {
            *journal.borrow_mut() = true;
            Ok(())
        },
        || Ok(disks.clone()),
        |_| {
            *ouverte.borrow_mut() = true;
            fenetre
        },
    );
    Resultat {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
        journal_installe: *journal.borrow(),
        fenetre_ouverte: *ouverte.borrow(),
    }
}

/// Lance la commande avec des disques fixes ; renvoie code, sortie standard, sortie d'erreur.
fn lancer(args: &[&str], disks: Vec<Disk>) -> (u8, String, String) {
    let r = lancer_avec(args, None, disks, Ok(()));
    (r.code, r.out, r.err)
}

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

/// Un verrou de la sortie d'erreur tenu pendant que la fenêtre est ouverte bloquerait pour toujours
/// tout autre fil qui journalise (énumération, minuteur) sous `bonk -v`.
#[test]
fn un_autre_fil_peut_journaliser_pendant_que_la_fenetre_est_ouverte() {
    // Élevé (runner Windows), la fenêtre refuse de s'ouvrir avant d'atteindre ce qu'on teste ici.
    if bonk_hal::platform::current().is_privileged() {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let code = run(["bonk", "-v", "gui"], |_| {
            std::thread::spawn(|| {
                log::warn!(target: "bonk::gui", "journal depuis un autre fil");
            })
            .join()
            .map_err(|_| "le fil a paniqué".to_owned())
        });
        let _ = tx.send(code == ExitCode::SUCCESS);
    });
    assert_eq!(
        rx.recv_timeout(std::time::Duration::from_secs(10)),
        Ok(true),
        "blocage : le journal d'un autre fil attend le verrou de stderr"
    );
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

/// Lance une commande avec un fichier de configuration propre au test et des variables injectées.
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
    // Sans option, `list` suit le réglage et montre aussi les disques écartés.
    let r = lancer_config(&["list"], &file, &[], sample());
    assert!(r.out.contains("nvme-systeme"), "{}", r.out);
    // La variable d'environnement l'emporte sur le fichier.
    let r = lancer_config(
        &["list"],
        &file,
        &[("BONK_VIEW_SHOW_ALL", "false")],
        sample(),
    );
    assert!(!r.out.contains("nvme-systeme"), "{}", r.out);
    // La fenêtre démarre avec ces réglages.
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let vue = RefCell::new(ViewDefaults::default());
    let env = Env {
        config_path: Some(file.clone()),
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
            *vue.borrow_mut() = v;
            Ok(())
        },
    );
    assert_eq!(code, 0);
    assert!(vue.borrow().show_all && !vue.borrow().show_internal);
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
    // `config list` en revanche le dit.
    let r = lancer_config(&["config", "list"], &file, &[], vec![]);
    assert_eq!(r.code, EXIT_USAGE);
    assert!(r.err.contains("unreadable"), "{}", r.err);
    let _ = std::fs::remove_dir_all(file.parent().ok_or("parent")?);
    Ok(())
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

/// La sortie réelle de `list --json` respecte son propre schéma : mêmes champs de premier niveau.
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

#[test]
fn sans_commande_la_fenetre_s_ouvre() {
    let r = lancer_avec(&[], None, vec![], Ok(()));
    assert_eq!(r.code, 0);
    assert!(r.fenetre_ouverte && r.journal_installe);
    assert!(r.out.is_empty());
}

#[test]
fn verbeux_sans_commande_ouvre_la_fenetre_avec_le_journal() {
    for args in [&["-v"][..], &["-vv"], &["gui", "-v"]] {
        let r = lancer_avec(args, None, vec![], Ok(()));
        assert_eq!(r.code, 0, "{args:?}");
        assert!(r.fenetre_ouverte && r.journal_installe, "{args:?}");
    }
}

#[test]
fn l_echec_de_la_fenetre_renvoie_vers_la_ligne_de_commande() {
    let r = lancer_avec(&[], None, vec![], Err("aucun affichage".to_owned()));
    assert_eq!(r.code, EXIT_RUNTIME_ERROR);
    assert!(
        r.err
            .starts_with("bonk: error: could not open the graphical interface: aucun affichage"),
        "{}",
        r.err
    );
    assert!(r.err.contains("bonk --help"), "{}", r.err);
}

#[test]
fn les_options_de_journal_sont_acceptees_apres_la_commande() {
    let r = lancer_avec(
        &[
            "list",
            "-vv",
            "--log-format",
            "json",
            "--log-file",
            "/tmp/x.log",
        ],
        None,
        sample(),
        Ok(()),
    );
    assert_eq!(r.code, 0, "{}", r.err);
    assert!(r.journal_installe);
}

#[test]
fn un_reglage_de_journal_invalide_est_une_erreur_d_usage() {
    let r = lancer_avec(&["list"], Some("gui=bavard"), sample(), Ok(()));
    assert_eq!(r.code, EXIT_USAGE);
    assert!(r.err.contains("unknown log level"), "{}", r.err);
    assert!(!r.journal_installe && r.out.is_empty());
}

#[test]
fn un_journal_impossible_a_installer_est_une_erreur_d_execution() {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let code = execute(
        ["bonk", "list"],
        &Env::default(),
        &mut out,
        &mut err,
        |_| Err("journal : ouverture impossible".to_owned()),
        || Ok(vec![]),
        |_| Ok(()),
    );
    assert_eq!(code, EXIT_RUNTIME_ERROR);
    assert!(out.is_empty());
}
