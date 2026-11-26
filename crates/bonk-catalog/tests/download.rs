#![allow(clippy::unwrap_used)]

mod support;

use std::cell::Cell;
use std::fs;
use std::io::Write;
use std::time::Duration;

use bonk_catalog::{Advance, DownloadError, Hooks, download, fetch_limited, fetch_text, is_url};
use support::*;

fn run(url: &str, dest: &std::path::Path) -> Result<u64, DownloadError> {
    run_with(url, dest, &|| false, Duration::from_secs(20))
}

fn run_with(
    url: &str,
    dest: &std::path::Path,
    cancelled: &dyn Fn() -> bool,
    silence_limit: Duration,
) -> Result<u64, DownloadError> {
    let mut progress = |_: Advance| {};
    download(
        url,
        dest,
        &mut Hooks {
            progress: &mut progress,
            cancelled,
            pause: &|_| {},
            silence_limit,
        },
    )
}

#[test]
fn un_telechargement_complet_porte_le_nom_final() {
    let body = data(1 << 20);
    let server = Server::start(body.clone(), |_| Reply::Serve);
    let temp = Temp::new("complet");
    assert_eq!(run(&server.url(), &temp.dest()).unwrap(), 1 << 20);
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
    assert!(!temp.part().exists());
}

#[test]
fn une_coupure_reprend_seule_sans_tout_redemander() {
    let body = data(3 << 20);
    let server = Server::start(body.clone(), |n| match n {
        0 => Reply::CutAfter(1 << 20),
        _ => Reply::Serve,
    });
    let temp = Temp::new("coupure");
    run(&server.url(), &temp.dest()).unwrap();
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requested_start(&requests[1]).is_some_and(|start| start > 0),
        "{requests:?}"
    );
    assert!(
        requests[1].contains(&format!("if-range: {ETAG}")),
        "{requests:?}"
    );
}

#[test]
fn un_fichier_partiel_reste_apres_un_echec_et_sert_la_fois_suivante() {
    let body = data(1 << 20);
    let cut = 400_000;
    let first = Server::start(body.clone(), move |n| match n {
        0 => Reply::CutAfter(cut),
        _ => Reply::Status(503),
    });
    let temp = Temp::new("deux-fois");
    assert!(matches!(
        run(&first.url(), &temp.dest()),
        Err(DownloadError::Network(_))
    ));
    assert!(!temp.dest().exists());
    let kept = fs::metadata(temp.part()).unwrap().len();
    assert!(kept > 0);

    let second = Server::start(body.clone(), |_| Reply::Serve);
    run(&second.url(), &temp.dest()).unwrap();
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
    assert_eq!(
        requested_start(&second.requests()[0]),
        usize::try_from(kept).ok()
    );
}

#[test]
fn un_fichier_change_sur_le_serveur_repart_de_zero() {
    let body = data(1 << 20);
    let server = Server::start(body.clone(), |_| Reply::Serve);
    let temp = Temp::new("change");
    fs::write(temp.part(), b"ancienne version").unwrap();
    fs::write(temp.0.join("image.iso.part.validateur"), "\"v0\"").unwrap();
    run(&server.url(), &temp.dest()).unwrap();
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
}

#[test]
fn un_serveur_sans_reprise_renvoie_tout_et_le_fichier_reste_juste() {
    let body = data(1 << 20);
    let server = Server::start(body.clone(), |_| Reply::IgnoreRange);
    let temp = Temp::new("sans-range");
    fs::write(temp.part(), &body[..1000]).unwrap();
    run(&server.url(), &temp.dest()).unwrap();
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
}

#[test]
fn une_reponse_partielle_mal_placee_est_refusee_sans_rien_garder() {
    let server = Server::start(data(4096), |_| Reply::WrongStart);
    let temp = Temp::new("mal-placee");
    fs::write(temp.part(), [0; 100]).unwrap();
    assert!(matches!(
        run(&server.url(), &temp.dest()),
        Err(DownloadError::Inconsistent(_))
    ));
    assert_eq!(fs::metadata(temp.part()).unwrap().len(), 0);
}

#[test]
fn un_fichier_partiel_deja_complet_se_termine_sans_rien_recevoir() {
    let body = data(4096);
    let server = Server::start(body.clone(), |_| Reply::Serve);
    let temp = Temp::new("deja-complet");
    fs::write(temp.part(), &body).unwrap();
    run(&server.url(), &temp.dest()).unwrap();
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
}

#[test]
fn une_page_absente_est_definitive_du_premier_coup() {
    let server = Server::start(Vec::new(), |_| Reply::Status(404));
    let temp = Temp::new("absente");
    assert!(matches!(
        run(&server.url(), &temp.dest()),
        Err(DownloadError::Refused { status: 404, .. })
    ));
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn un_serveur_muet_est_abandonne_apres_plusieurs_essais() {
    let server = Server::start(Vec::new(), |_| Reply::Mute);
    let temp = Temp::new("muet");
    let result = run_with(
        &server.url(),
        &temp.dest(),
        &|| false,
        Duration::from_millis(20),
    );
    assert!(matches!(result, Err(DownloadError::Network(_))));
}

#[test]
fn l_annulation_garde_le_fichier_partiel() {
    let body = data(8 << 20);
    let server = Server::start(body, |_| Reply::Serve);
    let temp = Temp::new("annule");
    let calls = Cell::new(0);
    let cancelled = || {
        calls.set(calls.get() + 1);
        calls.get() > 3
    };
    assert!(matches!(
        run_with(
            &server.url(),
            &temp.dest(),
            &cancelled,
            Duration::from_secs(20)
        ),
        Err(DownloadError::Cancelled)
    ));
    assert!(!temp.dest().exists());
    assert!(temp.part().exists());
}

#[test]
fn seul_https_est_admis_hors_de_cette_machine() {
    let temp = Temp::new("schema");
    for url in [
        "http://example.org/a.iso",
        "ftp://example.org/a.iso",
        "http://127.0.0.1.example.org/a.iso",
    ] {
        assert!(
            matches!(run(url, &temp.dest()), Err(DownloadError::Scheme(_))),
            "{url}"
        );
    }
    assert!(matches!(
        fetch_text("http://example.org/SHA256SUMS"),
        Err(DownloadError::Scheme(_))
    ));
}

#[test]
fn le_texte_d_empreintes_se_lit_et_une_page_absente_se_dit() {
    let server = Server::start(b"contenu".to_vec(), |n| match n {
        0 => Reply::Serve,
        _ => Reply::Status(404),
    });
    assert_eq!(fetch_text(&server.url()).unwrap(), "contenu");
    assert!(matches!(
        fetch_text(&server.url()),
        Err(DownloadError::Refused { status: 404, .. })
    ));
}

#[test]
fn un_fichier_du_catalogue_se_recoit_compresse_et_se_lit_tel_quel() {
    let body = b"{\"schema_version\":2}".repeat(50);
    let server = Server::start(body.clone(), |_| Reply::Gzip);
    assert_eq!(
        fetch_limited(&server.url(), body.len() as u64).unwrap(),
        body
    );
    assert!(server.requests()[0].contains("accept-encoding: gzip"));
}

#[test]
fn une_reponse_compressee_ne_depasse_pas_la_taille_attendue_une_fois_decompressee() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&vec![0u8; 1 << 20]).unwrap();
    let bomb = encoder.finish().unwrap();
    assert!(bomb.len() < 2048);
    let server = Server::start(Vec::new(), move |_| Reply::Raw(bomb.clone()));
    let error = fetch_limited(&server.url(), 4096).unwrap_err();
    assert!(error.to_string().contains("once decompressed"), "{error}");
}

#[test]
fn une_image_se_demande_sans_compression() {
    let body = data(4096);
    let server = Server::start(body.clone(), |_| Reply::Gzip);
    let temp = Temp::new("identite");
    assert_eq!(run(&server.url(), &temp.dest()).unwrap(), body.len() as u64);
    assert_eq!(fs::read(temp.dest()).unwrap(), body);
    assert!(server.requests()[0].contains("accept-encoding: identity"));
}

#[test]
fn un_fichier_publie_se_lit_jusqu_a_sa_taille_signee() {
    let body = data(1_429_504);
    let server = Server::start(body.clone(), |_| Reply::Serve);
    assert_eq!(
        fetch_limited(&server.url(), body.len() as u64).unwrap(),
        body
    );
    assert!(fetch_limited(&server.url(), body.len() as u64 - 1).is_err());
}

#[test]
fn une_adresse_web_se_reconnait_a_son_schema() {
    assert!(is_url("https://a.org/x") && is_url("http://a.org/x"));
    assert!(!is_url("/home/a/https://x") && !is_url("image.iso"));
}
