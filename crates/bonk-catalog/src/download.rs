//! Téléchargement reprenable d'une image.
//!
//! Les octets arrivent dans `<destination>.part`, renommé une fois complet : un fichier qui porte le nom
//! final est toujours entier. Une coupure n'efface rien ; la reprise demande la suite par `Range`, et
//! `If-Range` fait repartir de zéro si le fichier a changé sur le serveur entre-temps.
//!
//! La connexion vit sur un fil à part qui passe ses blocs par un canal : une lecture réseau bloquée
//! n'empêche ni l'annulation ni le constat d'un serveur muet. Seul l'appelant écrit dans le fichier :
//! un fil abandonné ne peut plus y toucher.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use ureq::http::{StatusCode, header};
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Proxy};

/// Essais sans un octet de plus avant d'abandonner.
const ATTEMPTS_WITHOUT_PROGRESS: u32 = 5;
/// Taille d'un bloc passé du fil réseau à l'écriture.
const CHUNK: usize = 256 * 1024;
/// Blocs en attente entre les deux fils : borne la mémoire si le disque est plus lent que le réseau.
const QUEUED_CHUNKS: usize = 8;
/// Fréquence à laquelle l'attente regarde si l'annulation a été demandée.
const POLL: Duration = Duration::from_millis(100);
/// Au-delà, un serveur qui n'envoie plus rien est quitté pour une nouvelle connexion.
pub const SILENCE_LIMIT: Duration = Duration::from_secs(60);
/// Un fichier d'empreintes tient largement là-dedans ; le plafond protège d'une URL qui se trompe.
const TEXT_LIMIT: u64 = 1 << 20;

#[derive(Debug)]
pub enum DownloadError {
    /// Ni `https://`, ni `http://` vers cette machine.
    Scheme(String),
    /// Réponse que réessayer ne changera pas.
    Refused {
        url: String,
        status: u16,
    },
    /// Réseau ou serveur, après tous les essais.
    Network(String),
    /// Le serveur répond autre chose que ce qui était demandé.
    Inconsistent(String),
    Io(io::Error),
    Cancelled,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheme(url) => write!(f, "{url}: only https:// is accepted"),
            Self::Refused { url, status } => write!(f, "{url}: the server answers {status}"),
            Self::Network(message) | Self::Inconsistent(message) => f.write_str(message),
            Self::Io(err) => write!(f, "download file: {err}"),
            Self::Cancelled => f.write_str("download cancelled, can be resumed"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<io::Error> for DownloadError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Avancement : octets présents dans le fichier, taille totale si le serveur la donne.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Advance {
    pub done: u64,
    pub total: Option<u64>,
}

/// Ce que l'appelant règle ; les délais sont courts dans les tests.
pub struct Hooks<'a> {
    /// Rappelé à chaque bloc écrit.
    pub progress: &'a mut dyn FnMut(Advance),
    pub cancelled: &'a dyn Fn() -> bool,
    /// Attente avant de réessayer, selon le numéro de l'essai.
    pub pause: &'a dyn Fn(u32),
    pub silence_limit: Duration,
}

/// `https://`, ou `http://` vers cette machine : les redirections sont soumises à la même règle.
fn https_required(url: &str) -> Result<bool, DownloadError> {
    if url.starts_with("https://") {
        return Ok(true);
    }
    let host = url.strip_prefix("http://").map(|rest| {
        let authority = rest.split('/').next().unwrap_or(rest);
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
    });
    match host {
        Some("127.0.0.1" | "localhost" | "[::1]") => Ok(false),
        _ => Err(DownloadError::Scheme(url.to_owned())),
    }
}

/// Une URL que Bonk téléchargerait, par opposition à un chemin de fichier.
#[must_use]
pub fn is_url(text: &str) -> bool {
    text.starts_with("https://") || text.starts_with("http://")
}

/// Nom du fichier désigné par une URL : le dernier segment du chemin, sans requête ni fragment. `None` s'il
/// est vide ou encodé : un nom venu du réseau ne doit pas pouvoir sortir du dossier de cache.
#[must_use]
pub fn file_name(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    let (_, after_scheme) = path.split_once("://")?;
    let (_, path) = after_scheme.split_once('/')?;
    let name = path.rsplit('/').next()?;
    let safe = !name.is_empty() && name != ".." && name != "." && !name.contains(['%', '\\', '\0']);
    safe.then_some(name)
}

/// Proxy de tout le processus. Un réglage unique : un téléchargement ne doit pas pouvoir l'oublier.
static PROXY: RwLock<Option<Proxy>> = RwLock::new(None);

/// Fait passer tout le réseau de Bonk par ce proxy. `socks5h://` seul : avec un autre schéma, les noms seraient
/// résolus hors du proxy, et l'anonymat promis par Tor serait faux. Un proxy injoignable donne une erreur,
/// jamais une connexion directe.
///
/// # Errors
/// Adresse mal formée, ou autre schéma que `socks5h://`.
pub fn set_proxy(url: &str) -> Result<(), String> {
    if !url.starts_with("socks5h://") {
        return Err(format!("{url}: only socks5h:// is accepted"));
    }
    let proxy = Proxy::new(url).map_err(|e| format!("{url}: {e}"))?;
    *PROXY.write().unwrap_or_else(PoisonError::into_inner) = Some(proxy);
    Ok(())
}

fn agent(https_only: bool) -> Result<Agent, DownloadError> {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    Ok(Agent::config_builder()
        .https_only(https_only)
        .proxy(PROXY.read().unwrap_or_else(PoisonError::into_inner).clone())
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        // 4xx et 5xx se lisent comme des réponses : leur traitement diffère.
        .http_status_as_error(false)
        .user_agent(concat!("bonk/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent())
}

/// Lit un petit texte (fichier d'empreintes).
///
/// # Errors
/// Adresse refusée, réponse autre que 200, réseau, texte trop long.
pub fn fetch_text(url: &str) -> Result<String, DownloadError> {
    String::from_utf8(fetch_bytes(url)?).map_err(|e| DownloadError::Network(format!("{url}: {e}")))
}

/// Lit un petit fichier tel quel (manifeste, signature, logo).
///
/// # Errors
/// Adresse refusée, réponse autre que 200, réseau, fichier trop long.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, DownloadError> {
    let agent = agent(https_required(url)?)?;
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| DownloadError::Network(format!("{url}: {e}")))?;
    let status = response.status();
    if status != StatusCode::OK {
        return Err(DownloadError::Refused {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }
    response
        .body_mut()
        .with_config()
        .limit(TEXT_LIMIT)
        .read_to_vec()
        .map_err(|e| DownloadError::Network(format!("{url}: {e}")))
}

/// Ce que dit le serveur en tête d'une réponse.
struct Head {
    status: u16,
    content_length: Option<u64>,
    content_range: Option<String>,
    validator: Option<String>,
}

enum Event {
    Head(Head),
    Chunk(Vec<u8>),
    End,
    Failed(String),
}

fn header_text(headers: &ureq::http::HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_owned)
}

/// Une réponse lue sur son fil : tête, blocs, puis `End` ou `Failed`. S'arrête dès que l'appelant lâche.
fn connect(agent: Agent, url: String, from: u64, validator: Option<String>) -> Receiver<Event> {
    let (sender, receiver) = mpsc::sync_channel(QUEUED_CHUNKS);
    std::thread::spawn(move || {
        if let Err(message) = read_response(&agent, &url, from, validator.as_deref(), &sender) {
            let _ = sender.send(Event::Failed(message));
        }
    });
    receiver
}

fn read_response(
    agent: &Agent,
    url: &str,
    from: u64,
    validator: Option<&str>,
    sender: &SyncSender<Event>,
) -> Result<(), String> {
    let mut request = agent.get(url);
    if from > 0 {
        request = request.header(header::RANGE, format!("bytes={from}-"));
        if let Some(validator) = validator {
            request = request.header(header::IF_RANGE, validator);
        }
    }
    let response = request.call().map_err(|e| format!("{url}: {e}"))?;
    let headers = response.headers();
    // Une ETag faible ne garantit pas les mêmes octets : `If-Range` ne l'admet pas.
    let validator = header_text(headers, header::ETAG)
        .filter(|tag| !tag.starts_with("W/"))
        .or_else(|| header_text(headers, header::LAST_MODIFIED));
    let head = Head {
        status: response.status().as_u16(),
        content_length: header_text(headers, header::CONTENT_LENGTH).and_then(|n| n.parse().ok()),
        content_range: header_text(headers, header::CONTENT_RANGE),
        validator,
    };
    if sender.send(Event::Head(head)).is_err() {
        return Ok(());
    }
    let mut body = response.into_body();
    let mut reader = body.with_config().limit(u64::MAX).reader();
    loop {
        let mut chunk = vec![0; CHUNK];
        let n = reader.read(&mut chunk).map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            let _ = sender.send(Event::End);
            return Ok(());
        }
        chunk.truncate(n);
        if sender.send(Event::Chunk(chunk)).is_err() {
            return Ok(());
        }
    }
}

/// Début et taille totale d'une réponse partielle : `bytes 100-199/200`, le total pouvant être `*`.
fn parse_content_range(value: &str) -> Option<(u64, Option<u64>)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.parse().ok()?, total.parse().ok()))
}

/// Total annoncé par une réponse 416 : `bytes */200`.
fn unsatisfied_total(value: &str) -> Option<u64> {
    value.strip_prefix("bytes */")?.parse().ok()
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

/// Fichier partiel et ce qui l'identifie sur le serveur.
struct Partial {
    file: File,
    done: u64,
    validator_path: PathBuf,
    validator: Option<String>,
}

impl Partial {
    fn open(part: &Path) -> io::Result<Self> {
        // Pas d'ouverture en ajout : Windows n'y permettrait pas de tronquer le fichier.
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(part)?;
        let done = file.seek(SeekFrom::End(0))?;
        let validator_path = with_suffix(part, ".validateur");
        let validator = fs::read_to_string(&validator_path).ok();
        Ok(Self {
            file,
            done,
            validator_path,
            validator,
        })
    }

    fn restart(&mut self) -> io::Result<()> {
        self.file.set_len(0)?;
        self.file.seek(SeekFrom::Start(0))?;
        self.done = 0;
        self.validator = None;
        match fs::remove_file(&self.validator_path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }

    fn remember(&mut self, validator: Option<String>) -> io::Result<()> {
        if let Some(validator) = &validator {
            fs::write(&self.validator_path, validator)?;
        }
        self.validator = validator;
        Ok(())
    }

    fn finish(self, part: &Path, dest: &Path) -> io::Result<()> {
        self.file.sync_all()?;
        // Fermé avant de renommer : Windows refuse de renommer un fichier ouvert.
        drop(self.file);
        fs::rename(part, dest)?;
        match fs::remove_file(&self.validator_path) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        }
    }
}

/// Fin d'un essai.
enum Attempt {
    Complete,
    /// À réessayer : le message dit pourquoi.
    Retry(String),
}

/// Télécharge `url` vers `dest`, en reprenant `<dest>.part` s'il existe. Réessaie seul tant que le réseau
/// donne de nouveaux octets ; après une coupure définitive, le fichier partiel reste pour la fois suivante.
///
/// # Errors
/// Adresse refusée, réponse refusée, réseau après tous les essais, écriture locale, annulation.
pub fn download(url: &str, dest: &Path, hooks: &mut Hooks<'_>) -> Result<u64, DownloadError> {
    let agent = agent(https_required(url)?)?;
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let part = with_suffix(dest, ".part");
    let mut partial = Partial::open(&part)?;
    let mut failures = 0;
    loop {
        let before = partial.done;
        match attempt(&agent, url, &mut partial, hooks)? {
            Attempt::Complete => {
                let done = partial.done;
                partial.finish(&part, dest)?;
                return Ok(done);
            }
            Attempt::Retry(message) => {
                failures = if partial.done > before {
                    1
                } else {
                    failures + 1
                };
                log::warn!(target: "bonk::catalog", attempt = failures, done = partial.done; "download interrupted: {message}");
                if failures >= ATTEMPTS_WITHOUT_PROGRESS {
                    return Err(DownloadError::Network(message));
                }
                (hooks.pause)(failures);
            }
        }
    }
}

fn attempt(
    agent: &Agent,
    url: &str,
    partial: &mut Partial,
    hooks: &mut Hooks<'_>,
) -> Result<Attempt, DownloadError> {
    let from = partial.done;
    let events = connect(
        agent.clone(),
        url.to_owned(),
        from,
        partial.validator.clone(),
    );
    let head = match next_event(&events, hooks)? {
        Next::Event(Event::Head(head)) => head,
        Next::Event(Event::Failed(message)) => return Ok(Attempt::Retry(message)),
        Next::Silent => {
            return Ok(Attempt::Retry(format!(
                "{url}: the server no longer answers"
            )));
        }
        Next::Event(_) | Next::Gone => {
            return Ok(Attempt::Retry(format!("{url}: response lost")));
        }
    };
    let total = match head.status {
        200 => {
            // Reprise refusée ou fichier changé : tout recommence.
            partial.restart()?;
            partial.remember(head.validator)?;
            head.content_length
        }
        206 => {
            let range = head.content_range.as_deref().and_then(parse_content_range);
            let Some((_, total)) = range.filter(|(start, _)| *start == from) else {
                partial.restart()?;
                return Err(DownloadError::Inconsistent(format!(
                    "{url}: partial response not starting at byte {from}"
                )));
            };
            if partial.validator.is_none() {
                partial.remember(head.validator)?;
            }
            total
        }
        416 => {
            let total = head.content_range.as_deref().and_then(unsatisfied_total);
            if total == Some(from) {
                return Ok(Attempt::Complete);
            }
            partial.restart()?;
            return Ok(Attempt::Retry(format!(
                "{url}: partial file longer than the original"
            )));
        }
        status if status >= 500 || status == 408 || status == 429 => {
            return Ok(Attempt::Retry(format!(
                "{url}: the server answers {status}"
            )));
        }
        status => {
            return Err(DownloadError::Refused {
                url: url.to_owned(),
                status,
            });
        }
    };
    loop {
        match next_event(&events, hooks)? {
            Next::Event(Event::Chunk(chunk)) => {
                partial.file.write_all(&chunk)?;
                partial.done += chunk.len() as u64;
                (hooks.progress)(Advance {
                    done: partial.done,
                    total,
                });
            }
            Next::Event(Event::End) => {
                return Ok(match total {
                    Some(total) if partial.done != total => {
                        Attempt::Retry(format!("{url}: {} bytes received of {total}", partial.done))
                    }
                    _ => Attempt::Complete,
                });
            }
            Next::Event(Event::Failed(message)) => return Ok(Attempt::Retry(message)),
            Next::Silent => {
                return Ok(Attempt::Retry(format!(
                    "{url}: the server sends nothing more"
                )));
            }
            Next::Event(Event::Head(_)) | Next::Gone => {
                return Ok(Attempt::Retry(format!("{url}: response lost")));
            }
        }
    }
}

enum Next {
    Event(Event),
    Silent,
    Gone,
}

fn next_event(events: &Receiver<Event>, hooks: &Hooks<'_>) -> Result<Next, DownloadError> {
    let mut waited = Duration::ZERO;
    loop {
        if (hooks.cancelled)() {
            return Err(DownloadError::Cancelled);
        }
        let step = POLL.min(hooks.silence_limit.saturating_sub(waited));
        match events.recv_timeout(step) {
            Ok(event) => return Ok(Next::Event(event)),
            Err(RecvTimeoutError::Disconnected) => return Ok(Next::Gone),
            Err(RecvTimeoutError::Timeout) => {
                waited += step;
                if waited >= hooks.silence_limit {
                    return Ok(Next::Silent);
                }
            }
        }
    }
}
