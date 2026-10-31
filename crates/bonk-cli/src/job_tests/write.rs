//! Écriture et vérification : ordre des garde-fous, images refusées, relecture du support.

use super::*;

#[test]
fn ecriture_complete_dans_l_ordre_des_garde_fous() {
    let source = TempSource::new("ordre", 1 << 20);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    // Démontage d'abord, ouverture en écriture ensuite, jamais l'inverse ; la table n'est relue qu'une
    // fois le disque refermé, sinon le noyau garde l'ancienne taille des partitions.
    assert_eq!(
        scenario.log(),
        [
            "prepare:/dev/sdb",
            "unmount:usb-a",
            "open:/dev/sdb:ReadWrite",
            "drop_cache",
            "rescan:/dev/sdb"
        ]
    );
    assert_eq!(scenario.written(1 << 20), source.bytes());
    assert!(out.contains("Démonté : /run/media/u/A"), "{out}");
    assert!(out.contains("Vérification : réussie"), "{out}");
}

#[test]
fn une_table_non_relue_n_enleve_rien_a_l_ecriture() {
    let source = TempSource::new("table-non-relue", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.rescan_fails = true;
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        scenario.log().contains(&"rescan:/dev/sdb".to_owned()),
        "{:?}",
        scenario.log()
    );
    assert_eq!(scenario.written(1 << 20), source.bytes());
}

#[test]
fn une_ecriture_simulee_ne_fait_relire_aucune_table() {
    let source = TempSource::new("simulee-source", 4096);
    let cible = TempSource::from_bytes("simulee-cible", &[0; 4096]);
    let scenario = Scenario::new();
    let args = ["write", "--simulate", &cible.arg(), &source.arg()];
    let (code, plan, err) = scenario.run(&[&args[..], &["--dry-run"]].concat());
    assert_eq!((code, err.as_str()), (0, ""), "{plan}");
    assert!(!plan.contains("reread_table"), "{plan}");
    let (code, out, err) = scenario.run(&[&args[..], &["--yes", "--progress", "none"]].concat());
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        !scenario.log().iter().any(|call| call.starts_with("rescan")),
        "{:?}",
        scenario.log()
    );
}

#[test]
fn sans_yes_rien_n_est_touche() {
    let source = TempSource::new("sans-yes", 4096);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["write", "usb-a", &source.arg()]);
    assert_eq!(code, EXIT_USAGE);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2004]"),
        "{err}"
    );
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn dry_run_decrit_sans_rien_faire() {
    let source = TempSource::new("dry", 4096);
    let scenario = Scenario::new();
    let (code, out, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Plan (rien n'est écrit)") && out.contains("Démontage : /run/media/u/A"),
        "{out}"
    );
    assert!(
        out.contains("unmount, write, flush, verify, repair_gpt_if_needed, reread_table"),
        "{out}"
    );
    assert!(scenario.log().is_empty());
    let (_, json, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("JSON valide");
    assert_eq!(value["event"], "plan");
    assert_eq!(value["target"]["id"], "usb-a");
    assert_eq!(value["unmount"][0], "/run/media/u/A");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_disque_interdit_est_refuse_avant_toute_action() {
    let source = TempSource::new("interdit", 4096);
    let scenario = Scenario::new();
    // Même avec --yes et toutes les options ouvertes.
    let (code, out, err) = scenario.run(&[
        "write",
        "nvme-sys",
        &source.arg(),
        "--yes",
        "--show-internal",
        "--allow-virtual",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2001]"),
        "{err}"
    );
    assert!(err.contains("volume système"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn seul_l_identifiant_stable_designe_une_cible() {
    let source = TempSource::new("chemin", 4096);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "/dev/sdb", &source.arg(), "--yes"]);
    assert_eq!(code, 1);
    assert!(err.trim_end().ends_with("[E1003]"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_disque_remplace_entre_les_deux_enumerations_n_est_pas_ecrit() {
    let source = TempSource::new("remplace", 4096);
    let mut scenario = Scenario::new();
    // Première énumération : la clé choisie. Seconde (juste avant l'ouverture) : un autre numéro de série.
    scenario.enumerations = vec![vec![usb("S1")], vec![usb("S2")]];
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2002]"),
        "{err}"
    );
    // Le démontage a eu lieu, l'ouverture jamais.
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
    assert!(scenario.written(4096).iter().all(|b| *b == 0));
}

#[test]
fn un_disque_devenu_interdit_entre_les_deux_enumerations_n_est_pas_ecrit() {
    let source = TempSource::new("devenu", 4096);
    let mut scenario = Scenario::new();
    let mut now_system = usb("S1");
    now_system.partitions = vec![partition("/dev/sdb1", &["/"], true)];
    scenario.enumerations = vec![vec![usb("S1")], vec![now_system]];
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(err.trim_end().ends_with("[E2001]"), "{err}");
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
}

#[test]
fn echec_de_demontage_empeche_l_ouverture() {
    let source = TempSource::new("demontage", 4096);
    let mut scenario = Scenario::new();
    scenario.unmount = Err("démontage de /run/media/u/A impossible : target is busy".to_owned());
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, 1);
    assert!(
        err.contains("busy") && err.trim_end().ends_with("[E2003]"),
        "{err}"
    );
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
}

#[test]
fn droits_insuffisants_a_l_ouverture() {
    let source = TempSource::new("droits", 4096);
    let mut scenario = Scenario::new();
    scenario.open = Err(io::ErrorKind::PermissionDenied);
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, 1);
    assert!(err.trim_end().ends_with("[E4004]"), "{err}");
}

#[test]
fn image_connue_trop_grande_refusee_avant_le_demontage() {
    let source = TempSource::new("grande", DISK_SIZE + 512);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_SOURCE);
    assert!(err.trim_end().ends_with("[E3003]"), "{err}");
    assert!(
        scenario.log().is_empty(),
        "rien ne doit être démonté pour une image qui ne tient pas"
    );
}

#[test]
fn annulation_avant_toute_ecriture() {
    let source = TempSource::new("annule", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.cancel = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_CANCELLED);
    assert!(err.trim_end().ends_with("[E4003]"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}

#[test]
fn une_relecture_fausse_fait_echouer_la_verification() {
    let source = TempSource::new("defectueux", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.faulty_reads = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
    // Sans vérification, la même écriture « réussit » : c'est bien la relecture qui détecte.
    let (code, _, _) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
        "--no-verify",
    ]);
    assert_eq!(code, 0);
}

#[test]
fn la_commande_verify_compare_sans_ecrire() {
    let source = TempSource::new("verify", 1 << 20);
    let scenario = Scenario::new();
    scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    scenario.log.borrow_mut().clear();
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("Vérification réussie"), "{out}");
    // Relecture seule : pas de démontage, ouverture en lecture.
    assert_eq!(
        scenario.log(),
        ["prepare:/dev/sdb", "open:/dev/sdb:ReadOnly", "drop_cache"]
    );
    // Un octet qui change : la comparaison échoue avec le code dédié.
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(0, &[0xAA; 512])
        .expect("écriture de test");
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
}

#[test]
fn verify_json_et_progression_json() {
    let source = TempSource::new("json", 2 << 20);
    let scenario = Scenario::new();
    let (code, out, _) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "json",
    ]);
    assert_eq!(code, 0);
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
        .collect();
    let (last, progress) = lines.split_last().expect("des lignes");
    assert_eq!(last["event"], "result");
    assert_eq!(last["write"]["bytes"], 2 << 20);
    assert!(
        progress.iter().all(|e| e["event"] == "progress"),
        "{progress:?}"
    );
    assert!(
        progress.iter().any(|e| e["phase"] == "write")
            && progress.iter().any(|e| e["phase"] == "verify")
    );
    let done: Vec<u64> = progress
        .iter()
        .filter(|e| e["phase"] == "write")
        .map(|e| e["bytes_done"].as_u64().unwrap_or(0))
        .collect();
    assert!(done.windows(2).all(|w| w[0] <= w[1]), "{done:?}");
}

/// La réparation du GPT modifie le MBR de protection et l'en-tête principal : `verify` de la même image
/// doit pourtant réussir, et non rendre `E4002`, quand le contenu est intact.
#[test]
fn verify_d_une_image_gpt_reussit_apres_la_reparation_du_gpt() {
    use bonk_core::partition::testing::{LINUX, gpt_image};

    let image = gpt_image(2048, &[(64, 1023, LINUX, "donnees")]);
    let source = TempSource::from_bytes("gpt-verify", &image);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("GPT : réparé"), "{out}");
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    // Un octet modifié hors des champs que la réparation réécrit reste détecté.
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(4096, &[0xAA; 512])
        .expect("écriture de test");
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
}

/// Sans vidage du cache avant la relecture, le contrôle lit le noyau et non le support (2 193 Mo/s
/// mesurés sur une clé USB 2.0) : une écriture perdue par la clé passerait inaperçue.
#[test]
fn la_relecture_de_controle_ne_lit_jamais_le_cache() {
    let source = TempSource::new("cache", 1 << 20);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
        "--no-repair-gpt",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        !scenario
            .log()
            .contains(&"lecture_avant_vidage_du_cache".to_owned()),
        "{:?}",
        scenario.log()
    );
    scenario.log.borrow_mut().clear();
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        scenario.log(),
        ["prepare:/dev/sdb", "open:/dev/sdb:ReadOnly", "drop_cache"]
    );
}

#[test]
fn un_refus_d_autorisation_ne_demonte_rien() {
    let source = TempSource::new("refus", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.prepare = Err("autorisation annulée".to_owned());
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("[E4004]"), "{err}");
    assert!(err.contains("autorisation annulée"), "{err}");
    let log = scenario.log.borrow();
    assert_eq!(*log, ["prepare:/dev/sdb"], "ni démontage ni ouverture");
}

/// Gzip fait à la main, blocs non compressés : le contenu est connu sans outil externe.
fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
    let mut chunks = data.chunks(0xFFFF).peekable();
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(chunk);
    }
    out.extend(bonk_core::partition::crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

/// ISO hybride qui annonce `sectors` secteurs de 2 Kio, dans une image de 1 Mio.
fn cut_iso(sectors: u32) -> Vec<u8> {
    let mut image: Vec<u8> = (0..1usize << 20).map(|i| (i % 251) as u8).collect();
    image[446..510].fill(0);
    image[510] = 0x55;
    image[511] = 0xAA;
    let primary = 16 * 2048;
    image[primary] = 1;
    image[primary + 1..primary + 6].copy_from_slice(b"CD001");
    image[primary + 80..primary + 84].copy_from_slice(&sectors.to_le_bytes());
    image[primary + 128..primary + 130].copy_from_slice(&2048u16.to_le_bytes());
    image
}

#[test]
fn image_compressee_coupee_a_l_interieur_n_aboutit_pas_a_un_succes() {
    // Annonce 4 Mio pour 1 Mio réel : tient sur la cible, flux gzip complet.
    let source = TempSource::from_bytes("coupee-gz", &gzip_stored(&cut_iso(2048)));
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3007"), "{err}");
    // Le mode brut écrit ce qu'on lui donne, en le journalisant.
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes", "--mode", "raw"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn image_compressee_annoncee_plus_grande_que_la_cible_ne_touche_a_rien() {
    let source = TempSource::from_bytes("grande-gz", &gzip_stored(&cut_iso(100_000)));
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3003"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    let (code, _, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert_eq!(code, EXIT_SOURCE);
}

#[test]
fn image_vide_refusee_meme_en_mode_brut() {
    for source in [
        TempSource::from_bytes("vide", &[]),
        TempSource::from_bytes("vide-gz", &gzip_stored(&[])),
    ] {
        let scenario = Scenario::new();
        let (code, _, err) =
            scenario.run(&["write", "usb-a", &source.arg(), "--yes", "--mode", "raw"]);
        assert_eq!(code, EXIT_SOURCE, "{err}");
        assert!(scenario.log().is_empty());
    }
}

#[test]
fn annuler_pendant_l_autorisation_laisse_la_cle_montee() {
    let source = TempSource::new("annule-autorisation", 4096);
    let mut scenario = Scenario::new();
    scenario.cancel_on_prepare = true;
    let (code, _, _) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_CANCELLED);
    assert_eq!(scenario.log(), ["prepare:/dev/sdb"]);
}

#[test]
fn le_refus_d_elever_va_jusqu_au_demontage() {
    let source = TempSource::new("sans-elevation", 4096);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--elevate",
        "never",
    ]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(scenario.seen_elevation.get(), Some(Elevation::Never));
}

#[test]
fn sans_reparation_demandee_le_gpt_n_est_pas_dit_sain() {
    let source = TempSource::new("sans-gpt", 4096);
    let (code, out, err) =
        Scenario::new().run(&["write", "usb-a", &source.arg(), "--yes", "--no-repair-gpt"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("GPT : contrôle non demandé"), "{out}");
    assert!(!out.contains("rien à réparer"), "{out}");
}

/// Le fichier est remplacé après son ouverture : l'empreinte porte sur les octets qui seront écrits,
/// pas sur ce que le chemin désigne au moment du contrôle.
#[test]
fn l_empreinte_porte_sur_le_fichier_ecrit_meme_remplace_entre_temps() {
    let ecrit = TempSource::from_bytes("piege-empreinte", &[0xAB; 1 << 20]);
    let attendu = TempSource::new("attendu-empreinte", 1 << 20);
    let mut lecteur = std::fs::File::open(&attendu.0).unwrap();
    let empreinte = bonk_core::write::hash_stream(
        &mut lecteur,
        None,
        bonk_core::write::DEFAULT_CHUNK,
        &mut |_| bonk_core::write::Control::Continue,
    )
    .unwrap();
    let (de, vers) = (attendu.0.clone(), ecrit.0.clone());
    let mut scenario = Scenario::new();
    scenario.on_first_enumeration = Some(Rc::new(move || {
        std::fs::rename(&de, &vers).unwrap();
    }));
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &ecrit.arg(),
        "--yes",
        "--mode",
        "raw",
        "--sha256",
        &empreinte,
    ]);
    assert_eq!(code, EXIT_SOURCE, "{out}\n{err}");
    assert!(err.contains("E3005"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}

/// Taille d'une image plus longue que le début lu par l'analyse : seul un passage complet voit sa fin.
const LONG_IMAGE: usize = 6 << 20;

/// Sans empreinte, une image compressée est décompressée une fois à blanc : des données après le flux sont
/// refusées avant d'avoir effacé la clé.
#[test]
fn des_donnees_apres_le_flux_sont_refusees_avant_toute_ecriture() {
    let mut image = gzip_stored(&vec![0xAB; LONG_IMAGE]);
    image.extend_from_slice(&[1; 4096]);
    let source = TempSource::from_bytes("flux-suivi", &image);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_SOURCE, "{out}\n{err}");
    assert!(err.contains("E3004"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(4096).iter().all(|b| *b == 0));
}

/// Une image compressée saine passe : le passage à blanc ne consomme pas le flux de l'écriture.
#[test]
fn une_image_compressee_saine_s_ecrit_apres_le_passage_a_blanc() {
    let source = TempSource::from_bytes("flux-sain", &gzip_stored(&vec![0xAB; LONG_IMAGE]));
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(scenario.written(LONG_IMAGE).iter().all(|b| *b == 0xAB));
}

/// Serveur local d'un seul fichier, servi autant de fois que demandé.
fn serve_file(body: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&hits);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    (format!("http://127.0.0.1:{port}/image.img"), hits)
}

fn sha256_of(bytes: &[u8]) -> String {
    bonk_core::write::hash_stream(
        &mut std::io::Cursor::new(bytes),
        None,
        bonk_core::write::DEFAULT_CHUNK,
        &mut |_| bonk_core::write::Control::Continue,
    )
    .unwrap()
}

#[test]
fn une_adresse_se_telecharge_et_s_ecrit_puis_sert_le_cache() {
    let body = vec![0xCD; 1 << 20];
    let (url, hits) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let cache = TempSource::dir("cache-ok");
    let scenario = Scenario::new();
    let args = [
        "write",
        "usb-a",
        &url,
        "--sha256",
        &sha,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ];
    let (code, out, err) = scenario.run(&args);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert_eq!(scenario.written(1 << 20), body);
    let (code, _, err) = scenario.run(&args);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn une_empreinte_fausse_n_ecrit_rien_et_oublie_le_fichier() {
    let (url, hits) = serve_file(vec![0xCD; 1 << 20]);
    let faux = "0".repeat(64);
    let cache = TempSource::dir("cache-faux");
    let scenario = Scenario::new();
    let args = [
        "write",
        "usb-a",
        &url,
        "--sha256",
        &faux,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ];
    let (code, _, err) = scenario.run(&args);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3005"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
    let (code, _, _) = scenario.run(&args);
    assert_eq!(code, EXIT_SOURCE);
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn une_adresse_sans_empreinte_ou_sans_confirmation_ne_telecharge_rien() {
    let (url, hits) = serve_file(vec![0; 16]);
    let cache = TempSource::dir("cache-refus");
    let scenario = Scenario::new();
    let (code, _, err) =
        scenario.run(&["write", "usb-a", &url, "--yes", "--cache-dir", &cache.arg()]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("--sha256"), "{err}");
    let faux = "0".repeat(64);
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256",
        &faux,
        "--cache-dir",
        &cache.arg(),
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");
    let (code, _, err) = scenario.run(&[
        "write",
        "absent",
        &url,
        "--sha256",
        &faux,
        "--yes",
        "--cache-dir",
        &cache.arg(),
    ]);
    assert_ne!(code, 0, "{err}");
    assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
}

#[test]
fn l_empreinte_se_lit_dans_un_fichier_d_empreintes_local_ou_distant() {
    let body = vec![0xEF; 1 << 20];
    let (url, _) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let sums = TempSource::from_bytes("sommes", format!("{sha}  image.img\n").as_bytes());
    let cache = TempSource::dir("cache-sommes");
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256-file",
        &sums.arg(),
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert_eq!(scenario.written(1 << 20), body);

    let (sums_url, _) = serve_file(format!("{sha}  image.img\n").into_bytes());
    let cache = TempSource::dir("cache-sommes-url");
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256-file",
        &sums_url,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""));

    let vide = TempSource::from_bytes("sommes-vides", b"# rien\n");
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256-file",
        &vide.arg(),
        "--cache-dir",
        &cache.arg(),
        "--yes",
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("image.img"), "{err}");
}

#[test]
fn une_coupure_definitive_se_dit_par_son_code_sans_toucher_au_support() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://127.0.0.1:{}/image.img",
        listener.local_addr().unwrap().port()
    );
    drop(listener);
    let faux = "0".repeat(64);
    let cache = TempSource::dir("cache-coupe");
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256",
        &faux,
        "--cache-dir",
        &cache.arg(),
        "--yes",
    ]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3008"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn le_flux_json_finit_par_un_evenement_d_erreur_avec_son_code() {
    let source = TempSource::new("flux-erreur", 4096);
    let faux = "0".repeat(64);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--mode",
        "raw",
        "--sha256",
        &faux,
        "--progress",
        "json",
    ]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    let last: serde_json::Value = serde_json::from_str(out.lines().last().unwrap()).unwrap();
    assert_eq!(last["event"], "error", "{out}");
    assert_eq!(last["code"], "E3005", "{out}");
    assert!(err.contains("E3005"), "{err}");
}

/// Le cache se liste et se vide, et seulement ce qu'il a lui-même rangé : un fichier étranger reste.
#[test]
fn le_cache_se_liste_et_se_vide_sans_toucher_au_reste() {
    let body = vec![0xCD; 1 << 20];
    let (url, _) = serve_file(body.clone());
    let sha = sha256_of(&body);
    let cache = TempSource::dir("cache-commandes");
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &url,
        "--sha256",
        &sha,
        "--cache-dir",
        &cache.arg(),
        "--yes",
        "--mode",
        "raw",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("bonk cache clean"), "{out}");

    let images = std::path::Path::new(&cache.arg()).join("images");
    std::fs::write(images.join("a-moi.txt"), b"garde").unwrap();
    std::fs::create_dir_all(images.join("autre")).unwrap();

    let (code, out, _) = scenario.run(&["cache", "list", "--cache-dir", &cache.arg(), "--json"]);
    assert_eq!(code, 0, "{out}");
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["entries"].as_array().map(Vec::len), Some(1), "{out}");
    assert_eq!(value["entries"][0]["sha256"], sha);
    assert_eq!(value["total_bytes"], 1 << 20);

    let (code, out, _) =
        scenario.run(&["cache", "clean", "--cache-dir", &cache.arg(), "--dry-run"]);
    assert_eq!(code, 0, "{out}");
    assert!(images.join(&sha).exists());

    let (code, out, _) = scenario.run(&["cache", "clean", "--cache-dir", &cache.arg()]);
    assert_eq!(code, 0, "{out}");
    assert!(!images.join(&sha).exists());
    assert!(images.join("a-moi.txt").exists());
    assert!(images.join("autre").exists());
}
