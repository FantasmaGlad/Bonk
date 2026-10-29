//! Écriture, vérification et réparation du GPT de bout en bout, par le binaire, sur des fichiers image
//! (`--simulate`) : aucun disque n'est touché. Les formats compressés sont produits par les outils du
//! système (`gzip`, `xz`, `zstd`) quand ils sont installés ; sinon ces cas sont ignorés.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use bonk_core::partition::testing::{ESP, LINUX, gpt_image};

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

fn image(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 253) as u8).collect()
}

fn outil(nom: &str) -> bool {
    Command::new("sh")
        .args(["-c", &format!("command -v {nom}")])
        .output()
        .is_ok_and(|s| s.status.success())
}

fn texte(octets: &[u8]) -> String {
    String::from_utf8_lossy(octets).into_owned()
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
        out.contains("Vérification : réussie") && out.contains("Source :") && out.contains("(raw)"),
        "{out}"
    );

    // Comparaison ultérieure sans écriture.
    let verify = bonk(&[
        "verify",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--progress",
        "none",
    ])?;
    assert_eq!(verify.status.code(), Some(0), "{}", texte(&verify.stderr));
    // Un octet qui change : code 4 et son identifiant.
    let mut tampered = ecrit;
    tampered[1000] ^= 1;
    fs::write(&cible, tampered)?;
    let mismatch = bonk(&[
        "verify",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--progress",
        "none",
    ])?;
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
        if !outil(outil_nom) {
            eprintln!("{outil_nom} absent : format ignoré");
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
    Ok(())
}

#[test]
fn garde_fous_de_la_ligne_de_commande() -> Resultat {
    let dir = Dossier::new("gardes")?;
    let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
    fs::write(&source, image(1 << 20))?;
    let (s, c) = (chemin(&source), chemin(&cible));

    // Sans --yes : usage, rien n'est créé.
    let sortie = bonk(&["write", "--simulate", &c, &s, "--size", "2097152"])?;
    assert_eq!(sortie.status.code(), Some(2));
    assert!(texte(&sortie.stderr).trim_end().ends_with("[E2004]"));
    assert!(!cible.exists());

    // --dry-run : le plan, rien n'est créé.
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

    // Image plus grande que la cible : code 6, rien n'est créé.
    let sortie = bonk(&["write", "--simulate", &c, &s, "--size", "524288", "--yes"])?;
    assert_eq!(sortie.status.code(), Some(6));
    assert!(texte(&sortie.stderr).trim_end().ends_with("[E3003]"));
    assert!(!cible.exists());

    // Taille de la cible inconnue : usage clair.
    let sortie = bonk(&["write", "--simulate", &c, &s, "--yes"])?;
    // L'image brute donne sa taille : la cible est créée à cette taille.
    assert_eq!(sortie.status.code(), Some(0), "{}", texte(&sortie.stderr));
    assert_eq!(fs::metadata(&cible)?.len(), 1 << 20);

    // Format non pris en charge : code 6 et dit quoi faire.
    let bz = dir.file("src.bz2");
    fs::write(&bz, b"BZh91AY&SYxxxxxxxxxxxxxxxx")?;
    let sortie = bonk(&[
        "write",
        "--simulate",
        &dir.file("o2.img").display().to_string(),
        &chemin(&bz),
        "--size",
        "1048576",
        "--yes",
    ])?;
    assert_eq!(sortie.status.code(), Some(6));
    let err = texte(&sortie.stderr);
    assert!(
        err.contains("décompresser l'image") && err.trim_end().ends_with("[E3002]"),
        "{err}"
    );

    // Image illisible.
    let sortie = bonk(&["write", "--simulate", &c, "/n/existe/pas.img", "--yes"])?;
    assert_eq!(sortie.status.code(), Some(6));
    assert!(texte(&sortie.stderr).trim_end().ends_with("[E3001]"));

    // Un disque ne se désigne jamais par son chemin : aucun disque ne répond à « /dev/sda ».
    let sortie = bonk(&["write", "/dev/sda", &s, "--yes"])?;
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

#[test]
fn trace_io_journalise_les_acces_sans_les_donnees() -> Resultat {
    let dir = Dossier::new("trace")?;
    let (source, cible) = (dir.file("src.img"), dir.file("out.img"));
    fs::write(&source, image(1 << 20))?;
    let sortie = bonk(&[
        "-vv",
        "--log-format",
        "json",
        "write",
        "--simulate",
        &chemin(&cible),
        &chemin(&source),
        "--yes",
        "--progress",
        "none",
        "--trace-io",
    ])?;
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
    // Par défaut, aucun octet de l'image dans le journal ; avec =data, le début seulement.
    assert!(acces.iter().all(|e| e["fields"]["head"] == ""));
    let avec = bonk(&[
        "-vv",
        "--log-format",
        "json",
        "write",
        "--simulate",
        &chemin(&dir.file("o2.img")),
        &chemin(&source),
        "--yes",
        "--progress",
        "none",
        "--trace-io=data",
    ])?;
    assert!(
        texte(&avec.stderr).contains("\"head\":\"00070e151c23"),
        "début de l'image attendu"
    );
    Ok(())
}

#[test]
fn reparation_du_gpt_apres_ecriture_sur_un_support_plus_grand() -> Resultat {
    let dir = Dossier::new("gpt")?;
    let (source, cible) = (dir.file("gpt.img"), dir.file("out.img"));
    fs::write(
        &source,
        gpt_image(
            4096,
            &[(2048, 2559, ESP, "EFI"), (2560, 4000, LINUX, "racine")],
        ),
    )?;
    let (s, c) = (chemin(&source), chemin(&cible));
    // Cible deux fois plus grande que l'image : le GPT de secours n'est plus à la fin.
    let sortie = bonk(&[
        "write",
        "--simulate",
        &c,
        &s,
        "--size",
        "4194304",
        "--yes",
        "--progress",
        "none",
        "--json",
    ])?;
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

    // Sans réparation demandée, l'anomalie reste visible, et `repair-gpt` la corrige ensuite.
    let brute = dir.file("brute.img");
    let sortie = bonk(&[
        "write",
        "--simulate",
        &chemin(&brute),
        &s,
        "--size",
        "4194304",
        "--yes",
        "--progress",
        "none",
        "--no-repair-gpt",
    ])?;
    assert_eq!(sortie.status.code(), Some(0));
    let plan = bonk(&[
        "repair-gpt",
        "--simulate",
        &chemin(&brute),
        "--dry-run",
        "--json",
    ])?;
    let plan: serde_json::Value = serde_json::from_slice(&plan.stdout)?;
    assert_eq!(plan["needs_repair"], true);
    assert!(
        texte(&bonk(&["repair-gpt", "--simulate", &chemin(&brute)])?.stderr).contains("[E2004]")
    );
    let fait = bonk(&[
        "repair-gpt",
        "--simulate",
        &chemin(&brute),
        "--yes",
        "--json",
    ])?;
    assert_eq!(fait.status.code(), Some(0));
    let fait: serde_json::Value = serde_json::from_slice(&fait.stdout)?;
    assert_eq!(fait["needs_repair"], true);
    assert!(fait["findings_after"].as_array().is_some_and(|a| {
        a.iter()
            .all(|c| c != "gpt_backup_not_at_end" && c != "gpt_backup_missing")
    }));
    // Une seconde fois : rien à faire.
    let encore = bonk(&[
        "repair-gpt",
        "--simulate",
        &chemin(&brute),
        "--yes",
        "--json",
    ])?;
    let encore: serde_json::Value = serde_json::from_slice(&encore.stdout)?;
    assert_eq!(encore["needs_repair"], false);
    Ok(())
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
