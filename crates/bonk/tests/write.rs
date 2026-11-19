#![allow(clippy::too_many_lines, reason = "a scenario test reads in one piece")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bonk_core::partition::testing::{ESP, LINUX, gpt_image};

mod common;

type Resultat = Result<(), Box<dyn std::error::Error>>;

struct Dossier(PathBuf);

impl Dossier {
    fn new(label: &str) -> Result<Self, std::io::Error> {
        let path = std::env::temp_dir().join(format!("bonk-e2e-{}-{label}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path)?;
        Ok(Self(path))
    }
    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dossier {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn bonk(args: &[&str]) -> Result<Output, std::io::Error> {
    Command::new(env!("CARGO_BIN_EXE_bonk")).args(args).output()
}

/// Recognizable pattern with a boot signature: the analysis sees a disk image.
fn image(len: usize) -> Vec<u8> {
    let mut data: Vec<u8> = (0..len).map(|i| (i * 7 % 253) as u8).collect();
    if len >= 512 {
        data[446..510].fill(0);
        data[510] = 0x55;
        data[511] = 0xAA;
    }
    data
}

fn texte(octets: &[u8]) -> String {
    String::from_utf8_lossy(octets).into_owned()
}

/// `bonk verify` of a simulated target against its source, writing nothing.
fn verifier(cible: &Path, source: &Path) -> Result<Output, std::io::Error> {
    bonk(&[
        "verify",
        "--simulate",
        &chemin(cible),
        &chemin(source),
        "--progress",
        "none",
    ])
}

fn chemin(p: &Path) -> String {
    p.display().to_string()
}

#[test]
fn ecriture_simulee_d_une_image_brute_puis_verification() -> Resultat {
    let dir = Dossier::new("brute")?;
    let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
    let data = image(3 * 1024 * 1024 + 123);
    fs::write(&source, &data)?;
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--size",
        "0x500000",
        "--yes",
        "--progress",
        "none",
    ])?;
    assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
    let ecrit = fs::read(&cible)?;
    assert_eq!(ecrit.len(), 5 * 1024 * 1024);
    assert_eq!(&ecrit[..data.len()], data.as_slice());
    assert!(ecrit[data.len()..].iter().all(|b| *b == 0));
    let out = texte(&sortie.stdout);
    assert!(
        out.contains("Verification: passed") && out.contains("Source:") && out.contains("(raw)"),
        "{out}"
    );

    // Later comparison, writing nothing.
    let verify = verifier(&cible, &source)?;
    assert_eq!(verify.status.code(), Some(0), "{}", texte(&verify.stderr));
    // A changed byte: code 4 and its id.
    let mut tampered = ecrit;
    tampered[1000] ^= 1;
    fs::write(&cible, tampered)?;
    let mismatch = verifier(&cible, &source)?;
    assert_eq!(mismatch.status.code(), Some(4));
    assert!(texte(&mismatch.stderr).trim_end().ends_with("[E4002]"));
    Ok(())
}

#[test]
fn formats_compresses_produits_par_les_outils_du_systeme() -> Resultat {
    let dir = Dossier::new("formats")?;
    let source = dir.file("src.img");
    let data = image(2 * 1024 * 1024 + 7);
    fs::write(&source, &data)?;
    let mut testes = 0;
    for (outil_nom, extension) in [("gzip", "gz"), ("xz", "xz"), ("zstd", "zst")] {
        if !common::outil_disponible(outil_nom) {
            continue;
        }
        let packed = dir.file(&format!("src.img.{extension}"));
        let status = Command::new(outil_nom)
            .args(["-k", "-f", "-q", &chemin(&source)])
            .status()?;
        assert!(status.success(), "{outil_nom}");
        let cible = dir.file(&format!("out-{extension}.img"));
        let sortie = bonk(&[
            "write",
            "--simulate",
            &chemin(&cible),
            &chemin(&packed),
            "--size",
            "4194304",
            "--yes",
            "--progress",
            "none",
            "--json",
        ])?;
        assert_eq!(
            sortie.status.code(),
            Some(0),
            "{outil_nom} : {}",
            texte(&sortie.stderr)
        );
        let json: serde_json::Value = serde_json::from_slice(&sortie.stdout)?;
        assert_eq!(
            json["source"]["format"],
            match extension {
                "gz" => "gzip",
                "zst" => "zstd",
                _ => "xz",
            }
        );
        assert_eq!(json["write"]["bytes"], data.len());
        assert_eq!(
            &fs::read(&cible)?[..data.len()],
            data.as_slice(),
            "{outil_nom}"
        );
        testes += 1;
    }
    eprintln!("{testes} format(s) compressé(s) éprouvé(s)");
    // gzip is part of every system the tests run on: none at all means the loop proved nothing.
    assert!(testes > 0, "aucun outil de compression trouvé");
    Ok(())
}

/// A refused write leaves the target untouched: (label, extra arguments, exit code, error code).
const REFUS: [(&str, &[&str], i32, &str); 3] = [
    ("sans --yes", &["--size", "2097152"], 2, "[E2004]"),
    (
        "cible trop petite",
        &["--size", "524288", "--yes"],
        6,
        "[E3003]",
    ),
    ("source absente", &["--yes"], 6, "[E3001]"),
];

#[test]
fn garde_fous_de_la_ligne_de_commande() -> Resultat {
    for (cas, extra, code, erreur) in REFUS {
        let dir = Dossier::new("gardes")?;
        let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
        fs::write(&source, image(1 << 20))?;
        let source = if cas == "source absente" {
            "/n/existe/pas.img".to_owned()
        } else {
            chemin(&source)
        };
        let mut args = vec!["write", "--simulate"];
        let c = chemin(&cible);
        args.extend([c.as_str(), source.as_str()]);
        args.extend(extra);
        let sortie = bonk(&args)?;
        assert_eq!(sortie.status.code(), Some(code), "{cas}");
        assert!(texte(&sortie.stderr).trim_end().ends_with(erreur), "{cas}");
        assert!(!cible.exists(), "{cas}");
    }
    Ok(())
}

#[test]
fn le_plan_a_blanc_n_ecrit_rien_et_l_ecriture_confirmee_ecrit_tout() -> Resultat {
    let dir = Dossier::new("plan")?;
    let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
    fs::write(&source, image(1 << 20))?;
    let (s, c) = (chemin(&source), chemin(&cible));
    let sortie = bonk(&[
        "write",
        "--simulate",
        &c,
        &s,
        "--size",
        "2097152",
        "--dry-run",
        "--json",
    ])?;
    assert_eq!(sortie.status.code(), Some(0));
    let plan: serde_json::Value = serde_json::from_slice(&sortie.stdout)?;
    assert_eq!(plan["event"], "plan");
    assert_eq!(plan["steps"][0], "write");
    assert!(!cible.exists());
    let sortie = bonk(&["write", "--simulate", &c, &s, "--yes"])?;
    assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
    assert_eq!(fs::metadata(&cible)?.len(), 1 << 20);
    Ok(())
}

#[test]
fn une_image_bzip2_et_un_disque_nomme_par_son_chemin_sont_refuses() -> Resultat {
    let dir = Dossier::new("refus")?;
    let (source, bz) = (dir.file("src.img"), dir.file("src.bz2"));
    fs::write(&source, image(1 << 20))?;
    fs::write(&bz, b"BZh91AY&SYxxxxxxxxxxxxxxxx")?;
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&dir.file("o2.img")),
        &chemin(&bz),
        "--size",
        "1048576",
        "--yes",
    ])?;
    assert_eq!(sortie.status.code(), Some(6));
    let err = texte(&sortie.stderr);
    assert!(
        err.contains("decompress the image") && err.trim_end().ends_with("[E3002]"),
        "{err}"
    );
    // A disk is never named by its path: no disk answers to /dev/sda.
    let sortie = bonk(&["write", "/dev/sda", &chemin(&source), "--yes"])?;
    assert_ne!(sortie.status.code(), Some(0));
    Ok(())
}

#[test]
fn progression_json_puis_resultat() -> Resultat {
    let dir = Dossier::new("progress")?;
    let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
    fs::write(&source, image(9 << 20))?;
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--yes",
        "--progress",
        "json",
    ])?;
    assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
    let lines: Vec<serde_json::Value> = texte(&sortie.stdout)
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let (last, progress) = lines.split_last().ok_or("aucune ligne")?;
    assert_eq!(last["event"], "result");
    assert!(progress.len() >= 5, "{} événements", progress.len());
    assert!(
        progress
            .iter()
            .all(|e| e["event"] == "progress" && e["schema_version"] == 1)
    );
    Ok(())
}

/// A simulated write with the access log as JSON on standard error.
fn ecrire_trace(cible: &Path, source: &Path, trace: &str) -> Result<Output, std::io::Error> {
    bonk(&[
        "-vv",
        "--log-format",
        "json",
        "write",
        "--simulate",
        &chemin(cible),
        &chemin(source),
        "--yes",
        "--progress",
        "none",
        trace,
    ])
}

#[test]
fn trace_io_journalise_les_acces_sans_les_donnees() -> Resultat {
    let dir = Dossier::new("trace")?;
    let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
    fs::write(&source, image(1 << 20))?;
    let sortie = ecrire_trace(&cible, &source, "--trace-io")?;
    assert_eq!(sortie.status.code(), Some(0));
    let journal = texte(&sortie.stderr);
    let acces: Vec<serde_json::Value> = journal
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|e| e["target"] == "bonk_core::trace")
        .collect();
    assert!(
        acces
            .iter()
            .any(|e| e["fields"]["op"] == "write" && e["fields"]["offset"] == 0),
        "{journal}"
    );
    assert!(acces.iter().any(|e| e["fields"]["op"] == "read"));
    assert!(
        acces
            .iter()
            .all(|e| e["fields"]["crc32"].is_string() && e["fields"]["micros"].is_number())
    );
    // By default no image byte in the log; with =data, only the head.
    assert!(acces.iter().all(|e| e["fields"]["head"] == ""));
    let avec = ecrire_trace(&dir.file("o2.img"), &source, "--trace-io=data")?;
    assert!(
        texte(&avec.stderr).contains("\"head\":\"00070e151c23"),
        "début de l'image attendu"
    );
    Ok(())
}

/// The low-level reads trace like the rest: `--trace-io` is accepted by every command that reads a device.
#[test]
fn trace_io_suit_aussi_les_lectures_de_debug() -> Resultat {
    let dir = Dossier::new("trace-debug")?;
    let source = dir.file("src.img");
    fs::write(&source, image(1 << 20))?;
    for commande in [&["debug", "hexdump"][..], &["debug", "layout"][..]] {
        let mut args = vec!["-vv", "--log-format", "json"];
        args.extend_from_slice(commande);
        let chemin_source = chemin(&source);
        args.extend_from_slice(&[chemin_source.as_str(), "--trace-io=data"]);
        let sortie = bonk(&args)?;
        assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
        let journal = texte(&sortie.stderr);
        assert!(
            journal.contains("\"target\":\"bonk_core::trace\"")
                && journal.contains("\"op\":\"read\""),
            "{commande:?} : {journal}"
        );
    }
    Ok(())
}

#[test]
fn reparation_du_gpt_apres_ecriture_sur_un_support_plus_grand() -> Resultat {
    let dir = Dossier::new("gpt")?;
    let (source, cible) = (source_gpt(&dir)?, dir.file("out.img"));
    let c = chemin(&cible);
    let sortie = ecrire_gpt(&cible, &source, "--json")?;
    assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
    let json: serde_json::Value = serde_json::from_slice(&sortie.stdout)?;
    assert!(
        json["gpt_repair"]["actions"]
            .as_array()
            .is_some_and(|a| a.iter().any(|x| x == "backup_header_written")),
        "{json}"
    );
    assert_eq!(json["gpt_repair"]["backup_header_lba"], 8191);

    let layout = bonk(&["debug", "layout", &c, "--json"])?;
    let layout: serde_json::Value = serde_json::from_slice(&layout.stdout)?;
    let graves: Vec<&serde_json::Value> = layout["findings"].as_array().map_or(vec![], |f| {
        f.iter().filter(|x| x["severity"] != "info").collect()
    });
    assert!(graves.is_empty(), "{graves:?}");
    assert_eq!(layout["gpt"]["backup"]["read_at_lba"], 8191);
    Ok(())
}

/// Without the repair the anomaly stays visible, and `repair-gpt` fixes it afterwards, once.
#[test]
fn repair_gpt_repare_apres_coup_une_ecriture_sans_reparation() -> Resultat {
    let dir = Dossier::new("gpt-brut")?;
    let (source, brute) = (source_gpt(&dir)?, dir.file("brute.img"));
    let b = chemin(&brute);
    assert_eq!(
        ecrire_gpt(&brute, &source, "--no-repair-gpt")?
            .status
            .code(),
        Some(0)
    );
    let plan = bonk(&["repair-gpt", "--simulate", &b, "--dry-run", "--json"])?;
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout)?;
    assert_eq!(plan["needs_repair"], true);
    assert!(texte(&bonk(&["repair-gpt", "--simulate", &b])?.stderr).contains("[E2004]"));
    let fait = bonk(&["repair-gpt", "--simulate", &b, "--yes", "--json"])?;
    assert_eq!(fait.status.code(), Some(0));
    let fait: serde_json::Value = serde_json::from_slice(&fait.stdout)?;
    assert_eq!(fait["needs_repair"], true);
    assert!(fait["findings_after"].as_array().is_some_and(|a| {
        a.iter()
            .all(|c| c != "gpt_backup_not_at_end" && c != "gpt_backup_missing")
    }));
    let encore = bonk(&["repair-gpt", "--simulate", &b, "--yes", "--json"])?;
    let encore: serde_json::Value = serde_json::from_slice(&encore.stdout)?;
    assert_eq!(encore["needs_repair"], false);
    Ok(())
}

/// A two-partition GPT image of 2 MiB.
fn source_gpt(dir: &Dossier) -> Result<PathBuf, std::io::Error> {
    let source = dir.file("gpt.img");
    fs::write(
        &source,
        gpt_image(
            4096,
            &[(2048, 2559, ESP, "EFI"), (2560, 4000, LINUX, "racine")],
        ),
    )?;
    Ok(source)
}

/// Written to a target twice the image size: the backup GPT is no longer at the end.
fn ecrire_gpt(cible: &Path, source: &Path, option: &str) -> Result<Output, std::io::Error> {
    bonk(&[
        "write",
        "--simulate",
        &chemin(cible),
        &chemin(source),
        "--size",
        "4194304",
        "--yes",
        "--progress",
        "none",
        option,
    ])
}

#[test]
fn les_schemas_decrivent_les_resultats_d_ecriture() -> Resultat {
    for nom in ["write", "write-plan", "verify", "repair-gpt", "progress"] {
        let sortie = bonk(&["schema", nom])?;
        assert_eq!(sortie.status.code(), Some(0), "{nom}");
        let schema: serde_json::Value = serde_json::from_slice(&sortie.stdout)?;
        assert_eq!(schema["type"], "object", "{nom}");
    }
    Ok(())
}

#[test]
fn une_empreinte_fausse_refuse_l_image_avant_de_creer_la_cible() -> Resultat {
    let dossier = Dossier::new("empreinte-fausse")?;
    let source = dossier.file("image.img");
    std::fs::write(&source, image(1 << 20))?;
    let cible = dossier.file("cle.img");
    let fausse = "0".repeat(64);
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--sha256",
        &fausse,
        "--yes",
        "--progress",
        "none",
    ])?;
    assert_eq!(sortie.status.code(), Some(6));
    assert!(texte(&sortie.stderr).contains("[E3005]"), "{sortie:?}");
    assert!(!cible.exists(), "rien ne doit avoir été touché");
    Ok(())
}

#[test]
fn une_empreinte_juste_est_acceptee_et_annoncee() -> Resultat {
    let dossier = Dossier::new("empreinte-juste")?;
    let source = dossier.file("image.img");
    std::fs::write(&source, image(1 << 20))?;
    // Hash from a first unchecked write: the path under test is the checked one.
    let premiere = bonk(&[
        "write",
        "--simulate",
        &chemin(&dossier.file("premiere.img")),
        &chemin(&source),
        "--yes",
        "--json",
        "--progress",
        "none",
    ])?;
    let resultat: serde_json::Value = serde_json::from_slice(&premiere.stdout)?;
    let empreinte = resultat["write"]["sha256"]
        .as_str()
        .ok_or("empreinte absente")?
        .to_owned();
    let cible = dossier.file("cle.img");
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--sha256",
        &empreinte.to_uppercase(),
        "--yes",
        "--progress",
        "none",
    ])?;
    assert!(sortie.status.success(), "{sortie:?}");
    assert!(texte(&sortie.stdout).contains("File hash: matches"));
    Ok(())
}

#[test]
fn une_empreinte_mal_formee_est_une_erreur_d_usage() -> Resultat {
    let dossier = Dossier::new("empreinte-forme")?;
    let source = dossier.file("image.img");
    std::fs::write(&source, image(4096))?;
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&dossier.file("cle.img")),
        &chemin(&source),
        "--sha256",
        "abc",
        "--yes",
    ])?;
    assert_eq!(sortie.status.code(), Some(2));
    Ok(())
}

/// Structureless data, as a bad download gives.
fn donnees() -> Vec<u8> {
    vec![0x5A; 1 << 16]
}

#[test]
fn des_donnees_sans_amorcage_sont_refusees_sauf_en_mode_brut() -> Resultat {
    let dossier = Dossier::new("sans-amorcage")?;
    let source = dossier.file("image.iso");
    std::fs::write(&source, donnees())?;
    let cible = dossier.file("cle.img");
    let lancer = |mode: &str| {
        bonk(&[
            "write",
            "--simulate",
            &chemin(&cible),
            &chemin(&source),
            "--mode",
            mode,
            "--yes",
            "--progress",
            "none",
        ])
    };
    let refus = lancer("auto")?;
    assert_eq!(refus.status.code(), Some(6));
    assert!(texte(&refus.stderr).contains("[E3006]"), "{refus:?}");
    assert!(!cible.exists());
    assert!(lancer("raw")?.status.success());
    assert_eq!(std::fs::read(&cible)?, donnees());
    Ok(())
}

#[test]
fn une_image_tronquee_est_refusee_meme_avec_un_support_plus_grand() -> Resultat {
    let dossier = Dossier::new("tronquee")?;
    // A 16 MiB hybrid ISO of which 1 MiB is left.
    let mut data = image(1 << 20);
    let primaire = 16 * 2048;
    data[primaire] = 1;
    data[primaire + 1..primaire + 6].copy_from_slice(b"CD001");
    data[primaire + 80..primaire + 84].copy_from_slice(&8192u32.to_le_bytes());
    data[primaire + 128..primaire + 130].copy_from_slice(&2048u16.to_le_bytes());
    let source = dossier.file("coupee.iso");
    std::fs::write(&source, &data)?;
    let cible = dossier.file("cle.img");
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--size",
        "67108864",
        "--yes",
        "--progress",
        "none",
    ])?;
    assert_eq!(sortie.status.code(), Some(6));
    assert!(texte(&sortie.stderr).contains("[E3007]"), "{sortie:?}");
    assert!(!cible.exists());
    Ok(())
}
