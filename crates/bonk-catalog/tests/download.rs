//! Téléchargements contre un serveur HTTP local : coupures, reprises, réponses inattendues. Aucun accès au
//! réseau réel.

#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bonk_catalog::{Advance, DownloadError, Hooks, download, fetch_text, file_name, is_url};

const ETAG: &str = "\"v1\"";

/// Ce que le serveur fait d'une connexion, selon son numéro (à partir de 0).
#[derive(Clone, Copy, PartialEq)]
enum Reply {
    /// Répond normalement, en honorant `Range` et `If-Range`.
    Serve,
    /// Répond normalement, mais coupe après ce nombre d'octets du corps.
    CutAfter(usize),
    /// Ignore `Range`.
    IgnoreRange,
    /// Partielle, à un autre début que celui demandé.
    WrongStart,
    /// Accepte et ne dit plus rien.
    Mute,
    Status(u16),
}

struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(body: Vec<u8>, script: impl Fn(usize) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let count = AtomicUsize::new(0);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let n = count.fetch_add(1, Ordering::SeqCst);
                let reply = script(n);
                let text = read_request(&stream);
                seen.lock().unwrap().push(text.clone());
                let body = body.clone();
                std::thread::spawn(move || answer(stream, &text, &body, reply));
            }
        });
        Self { port, requests }
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}/image.iso", self.port)
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut text = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            return text;
        }
        text.push_str(&line.to_ascii_lowercase());
    }
}

fn requested_start(request: &str) -> Option<usize> {
    let line = request.lines().find(|l| l.starts_with("range: bytes="))?;
    line.trim_start_matches("range: bytes=")
        .trim_end_matches('-')
        .parse()
        .ok()
}

fn answer(mut stream: TcpStream, request: &str, body: &[u8], reply: Reply) {
    let start = requested_start(request);
    let fresh = request.contains(&format!("if-range: {ETAG}"));
    let honour = reply != Reply::IgnoreRange && (fresh || !request.contains("if-range:"));
    match reply {
        Reply::Mute => {
            std::thread::sleep(Duration::from_secs(5));
            return;
        }
        Reply::Status(status) => {
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
            return;
        }
        Reply::WrongStart => {
            let _ = write!(
                stream,
                "HTTP/1.1 206 P\r\ncontent-range: bytes 1-1/{}\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx",
                body.len()
            );
            return;
        }
        _ => {}
    }
    let served = match start {
        Some(start) if honour && start >= body.len() => {
            let _ = write!(
                stream,
                "HTTP/1.1 416 R\r\ncontent-range: bytes */{}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                body.len()
            );
            return;
        }
        Some(start) if honour => {
            let _ = write!(
                stream,
                "HTTP/1.1 206 P\r\netag: {ETAG}\r\ncontent-range: bytes {start}-{}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len() - 1,
                body.len(),
                body.len() - start
            );
            &body[start..]
        }
        _ => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\netag: {ETAG}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            body
        }
    };
    let served = match reply {
        Reply::CutAfter(n) => &served[..n.min(served.len())],
        _ => served,
    };
    let _ = stream.write_all(served);
}

struct Temp(PathBuf);

impl Temp {
    fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bonk-telechargement-{}-{tag}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    fn dest(&self) -> PathBuf {
        self.0.join("image.iso")
    }
    fn part(&self) -> PathBuf {
        self.0.join("image.iso.part")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

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
fn le_nom_de_fichier_d_une_url_ne_sort_pas_du_dossier() {
    assert_eq!(
        file_name("https://a.org/x/debian.iso?y=1#z"),
        Some("debian.iso")
    );
    for url in [
        "https://a.org/",
        "https://a.org",
        "https://a.org/x/..",
        "https://a.org/x/%2e%2e%2fetc",
        "https://a.org/a\\b",
    ] {
        assert_eq!(file_name(url), None, "{url}");
    }
    assert!(is_url("https://a.org/x") && is_url("http://a.org/x"));
    assert!(!is_url("/home/a/https://x") && !is_url("image.iso"));
}
