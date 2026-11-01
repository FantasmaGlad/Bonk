//! `bonk serve` : le service headless. Tout ce que fait la fenêtre, sans interface graphique : un autre
//! programme (outil de support, orchestrateur, test) pilote Bonk par un protocole JSON, une ligne par
//! message, sur l'entrée et la sortie standard.
//!
//! - **Requête** : `{"id": 1, "method": "disk.list", "params": {"all": true}}`.
//! - **Réponse** : `{"id": 1, "result": {...}}` ou `{"id": 1, "error": {"code": "E1003", ...}}`, avec
//!   les mêmes codes d'erreur et les mêmes formats que la CLI en `--json`.
//! - **Événements** (sans `id`) : `ready` au démarrage, `job_started`, `progress` (avec `request_id`)
//!   pendant une écriture, `disks_changed` quand la liste des disques change (`watch.start`).
//!
//! Les méthodes portent le nom de la commande : `disk.list` est `bonk disk list`. Une seule opération
//! d'écriture à la fois ; `cancel` arrête une écriture ou une vérification entre deux blocs, un
//! formatage entre deux étapes ; une réparation du GPT commencée, quelques secteurs, va à son terme : la
//! couper abîmerait la table. À la fermeture de l'entrée, l'opération
//! en cours est annulée et le service s'arrête ; sous Unix, Ctrl+C et SIGTERM font de même et le
//! service rend le code 5. Les mêmes garde-fous que la CLI s'appliquent : rien ne s'écrit sans
//! `"yes": true` ou `"force": true`, ni sur un disque que le filtre interdit.

mod jobs;
mod methods;
mod params;
mod protocol;
#[cfg(test)]
mod tests;

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bonk_hal::{Disk, EnumerateError};
use serde_json::{Value, json};

use crate::config::ViewDefaults;
use crate::errors::{Code, EXIT_CANCELLED};
use crate::slot::JobSlot;
use bonk_ops::job::Io;

use jobs::{JobHandle, Kind, WatchHandle, start_or_fail, start_watch, stop_watch};
use methods::quick;
use protocol::{Out, Request, failure};

pub(crate) use protocol::{schema_request, schema_response};

/// Version du protocole, incrémentée à chaque changement incompatible.
pub(crate) const PROTOCOL: u32 = 1;

pub(crate) type EnumerateFn = Arc<dyn Fn() -> Result<Vec<Disk>, EnumerateError> + Send + Sync>;

pub(crate) type IoFactory = Arc<dyn Fn(Arc<AtomicBool>) -> Io + Send + Sync>;

/// Ce dont le service a besoin du système : substituable pour les tests.
pub(crate) struct Config {
    pub(crate) enumerate: EnumerateFn,
    pub(crate) make_io: IoFactory,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) vars: Vec<(String, String)>,
    pub(crate) view: ViewDefaults,
    /// Ctrl+C ou SIGTERM reçu : le service annule l'opération en cours et s'arrête.
    pub(crate) interrupted: Arc<dyn Fn() -> bool + Send + Sync>,
}

/// Méthodes du service : le nom et la description vivent ici, et `handle` doit traiter chaque variante.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Method {
    Ping,
    Version,
    Methods,
    DiskList,
    DiskInfo,
    Diagnose,
    Explain,
    Schema,
    ConfigList,
    ConfigGet,
    ConfigSet,
    ConfigReset,
    DebugFilter,
    WatchStart,
    WatchStop,
    Cancel,
    DiskWrite,
    DiskVerify,
    DiskRepairGpt,
    DiskFormat,
    Capabilities,
    CatalogList,
}

#[derive(Default)]
struct State {
    /// Emplacement de l'opération longue : rendu avant la réponse (`slot.rs`), alors que
    /// `thread.is_finished()` ne devient vrai qu'après et ferait refuser à tort la requête suivante.
    slot: JobSlot,
    job: Option<JobHandle>,
    watch: Option<WatchHandle>,
}

fn handle(line: &str, cfg: &Config, out: &Out, state: &mut State) {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => {
            out.respond(
                &Value::Null,
                Err(failure(
                    Code::ProtocolError,
                    format!("unreadable request: {e}"),
                )),
            );
            return;
        }
    };
    log::debug!(target: "bonk::cli", method = request.method.as_str(); "service request");
    let id = &request.id;
    let Some(method) = Method::from_name(&request.method) else {
        out.respond(
            id,
            Err(failure(
                Code::ProtocolError,
                format!(
                    "unknown method: '{}' (see the 'methods' method)",
                    request.method
                ),
            )),
        );
        return;
    };
    match method {
        Method::DiskWrite => start_or_fail(Kind::Write, &request, cfg, out, state),
        Method::DiskVerify => start_or_fail(Kind::Verify, &request, cfg, out, state),
        Method::DiskRepairGpt => start_or_fail(Kind::Repair, &request, cfg, out, state),
        Method::DiskFormat => start_or_fail(Kind::Format, &request, cfg, out, state),
        Method::Cancel => {
            let running = state.slot.is_busy();
            if let (true, Some(job)) = (running, &state.job) {
                job.cancel.store(true, Ordering::SeqCst);
            }
            out.respond(id, Ok(json!({"cancelling": running})));
        }
        Method::WatchStart => out.respond(id, start_watch(&request.params, cfg, out, state)),
        Method::WatchStop => out.respond(
            id,
            Ok(json!({"watching": false, "was_watching": stop_watch(state)})),
        ),
        _ => out.respond(id, quick(method, &request.params, cfg)),
    }
}

/// Boucle du service : lit les requêtes jusqu'à la fin de l'entrée, puis annule l'opération en cours
/// et s'arrête. Rend le code de sortie.
pub(crate) fn serve(
    input: impl BufRead + Send + 'static,
    output: Box<dyn Write + Send>,
    cfg: &Config,
) -> u8 {
    let out = Out(Arc::new(Mutex::new(output)));
    out.send(&json!({"event": "ready", "protocol": PROTOCOL, "version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS}));
    let mut state = State::default();
    let lines = read_lines(input);
    let mut interrupted = false;
    loop {
        if (cfg.interrupted)() {
            interrupted = true;
            break;
        }
        match lines.recv_timeout(INTERRUPT_POLL) {
            Ok(line) if line.trim().is_empty() => {}
            Ok(line) => handle(&line, cfg, &out, &mut state),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    // Client parti ou arrêt demandé : l'opération en cours s'arrête entre deux blocs et répond.
    if let Some(job) = state.job.take() {
        job.cancel.store(true, Ordering::SeqCst);
        let _ = job.thread.join();
    }
    stop_watch(&mut state);
    if interrupted { EXIT_CANCELLED } else { 0 }
}

/// Fréquence à laquelle la boucle regarde si un arrêt a été demandé pendant qu'elle attend une requête.
const INTERRUPT_POLL: Duration = Duration::from_millis(100);

/// Lit l'entrée sur un fil à part : la boucle du service ne reste jamais bloquée sur une lecture et
/// voit un arrêt demandé même quand le client se tait.
fn read_lines(input: impl BufRead + Send + 'static) -> Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for line in input.lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    receiver
}

impl Method {
    const ALL: [Self; 22] = [
        Self::Ping,
        Self::Version,
        Self::Methods,
        Self::DiskList,
        Self::DiskInfo,
        Self::Diagnose,
        Self::Explain,
        Self::Schema,
        Self::ConfigList,
        Self::ConfigGet,
        Self::ConfigSet,
        Self::ConfigReset,
        Self::DebugFilter,
        Self::WatchStart,
        Self::WatchStop,
        Self::Cancel,
        Self::DiskWrite,
        Self::DiskVerify,
        Self::DiskRepairGpt,
        Self::DiskFormat,
        Self::Capabilities,
        Self::CatalogList,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Ping => "ping",
            Self::Version => "version",
            Self::Methods => "methods",
            Self::DiskList => "disk.list",
            Self::DiskInfo => "disk.info",
            Self::Diagnose => "diagnose",
            Self::Explain => "explain",
            Self::Schema => "schema",
            Self::ConfigList => "config.list",
            Self::ConfigGet => "config.get",
            Self::ConfigSet => "config.set",
            Self::ConfigReset => "config.reset",
            Self::DebugFilter => "debug.filter",
            Self::WatchStart => "watch.start",
            Self::WatchStop => "watch.stop",
            Self::Cancel => "cancel",
            Self::DiskWrite => "disk.write",
            Self::DiskVerify => "disk.verify",
            Self::DiskRepairGpt => "disk.repair_gpt",
            Self::DiskFormat => "disk.format",
            Self::Capabilities => "capabilities",
            Self::CatalogList => "catalog.list",
        }
    }

    /// Description rendue par la méthode `methods`.
    fn about(self) -> &'static str {
        match self {
            Self::Ping => "answers pong",
            Self::Version => "Bonk version, system, protocol",
            Self::Methods => "list of methods",
            Self::DiskList => "bonk disk list: disks and verdicts",
            Self::DiskInfo => "bonk disk info: detail of a disk",
            Self::Diagnose => "bonk diagnose: support report",
            Self::Explain => "bonk explain: meaning of an error code",
            Self::Schema => "bonk schema: JSON Schema of an output",
            Self::ConfigList => "bonk config list",
            Self::ConfigGet => "bonk config get",
            Self::ConfigSet => "bonk config set",
            Self::ConfigReset => "bonk config reset",
            Self::DebugFilter => "bonk debug filter: filter rules",
            Self::WatchStart => "disks_changed events on each list change",
            Self::WatchStop => "stops watching",
            Self::Cancel => "cancels the running write operation",
            Self::DiskWrite => "bonk disk write: writes an image (long operation)",
            Self::DiskVerify => "bonk disk verify: compares without writing (long operation)",
            Self::DiskRepairGpt => "bonk disk repair-gpt (long operation)",
            Self::DiskFormat => "bonk format: table and filesystem (long operation)",
            Self::Capabilities => "bonk capabilities: what this host allows",
            Self::CatalogList => "bonk catalog list: images of the signed catalog",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }
}
