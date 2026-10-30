//! `bonk serve` : le service headless (CDC §9.5). Tout ce que fait la fenêtre, sans interface
//! graphique : un programme (agent de support, orchestrateur, test) pilote Bonk par un protocole
//! JSON, une ligne par message, sur l'entrée et la sortie standard.
//!
//! - **Requête** : `{"id": 1, "method": "disk.list", "params": {"all": true}}`.
//! - **Réponse** : `{"id": 1, "result": {...}}` ou `{"id": 1, "error": {"code": "E1003", ...}}`, avec
//!   les mêmes codes d'erreur et les mêmes formats que la CLI en `--json`.
//! - **Événements** (sans `id`) : `ready` au démarrage, `job_started`, `progress` (avec `request_id`)
//!   pendant une écriture, `disks_changed` quand la liste des disques change (`watch.start`).
//!
//! Les méthodes portent le nom de la commande : `disk.list` est `bonk disk list`. Une seule opération
//! d'écriture à la fois ; `cancel` l'arrête entre deux blocs. À la fermeture de l'entrée, l'opération
//! en cours est annulée et le service s'arrête. Les mêmes garde-fous que la CLI s'appliquent : rien
//! ne s'écrit sans `"yes": true`, ni sur un disque que le filtre interdit.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use bonk_core::write::{Control, Progress};
use bonk_hal::{Disk, EnumerateError, FilterOptions};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{self, ConfigAction, ConfigError, ViewDefaults};
use crate::errors::{self, Code};
use crate::job::{self, Deps, Io, RepairJob, Target, TraceIo, VerifyJob, WriteJob};
use crate::slot::JobSlot;
use crate::{debug, diagnose, info, list, schema};

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
}

/// Une requête du protocole.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct Request {
    /// Identifiant libre, rendu tel quel dans la réponse.
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Erreur d'une réponse.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct ErrorBody {
    /// Code stable (`bonk explain <code>`).
    code: &'static str,
    exit_code: u8,
    message: String,
}

/// Une réponse : `result` ou `error`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Response {
    id: Value,
    result: Option<Value>,
    error: Option<ErrorBody>,
}

pub(crate) fn schema_request() -> Value {
    serde_json::to_value(schemars::schema_for!(Request)).unwrap_or_default()
}

pub(crate) fn schema_response() -> Value {
    serde_json::to_value(schemars::schema_for!(Response)).unwrap_or_default()
}

/// Méthodes du service : nom, la méthode lance-t-elle une opération longue ?
const METHODS: [(&str, &str); 19] = [
    ("ping", "répond pong"),
    ("version", "version de Bonk, système, protocole"),
    ("methods", "liste des méthodes"),
    ("disk.list", "bonk disk list : disques et verdicts"),
    ("disk.info", "bonk disk info : détail d'un disque"),
    ("diagnose", "bonk diagnose : rapport de support"),
    ("explain", "bonk explain : sens d'un code d'erreur"),
    ("schema", "bonk schema : schéma JSON d'une sortie"),
    ("config.list", "bonk config list"),
    ("config.get", "bonk config get"),
    ("config.set", "bonk config set"),
    ("config.reset", "bonk config reset"),
    ("debug.filter", "bonk debug filter : règles du filtre"),
    (
        "watch.start",
        "événements disks_changed à chaque changement de la liste",
    ),
    ("watch.stop", "arrête la surveillance"),
    ("cancel", "annule l'opération d'écriture en cours"),
    (
        "disk.write",
        "bonk disk write : écrit une image (opération longue)",
    ),
    (
        "disk.verify",
        "bonk disk verify : compare sans écrire (opération longue)",
    ),
    ("disk.repair_gpt", "bonk disk repair-gpt (opération longue)"),
];

struct Failure {
    code: Code,
    message: String,
}

fn failure(code: Code, message: impl std::fmt::Display) -> Failure {
    Failure {
        code,
        message: message.to_string(),
    }
}

/// Flux de sortie partagé : chaque message est une ligne écrite d'un seul tenant.
#[derive(Clone)]
struct Out(Arc<Mutex<Box<dyn Write + Send>>>);

impl Out {
    fn send(&self, value: &Value) {
        if let Ok(mut writer) = self.0.lock() {
            let _ = writeln!(writer, "{value}");
            let _ = writer.flush();
        }
    }

    fn respond(&self, id: &Value, outcome: Result<Value, Failure>) {
        let message = match outcome {
            Ok(result) => json!({"id": id, "result": result}),
            Err(f) => {
                json!({"id": id, "error": {"code": f.code.id(), "exit_code": f.code.exit(), "message": f.message}})
            }
        };
        self.send(&message);
    }
}

// ---------------------------------------------------------------------------------------------
// Paramètres
// ---------------------------------------------------------------------------------------------

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ViewParams {
    all: bool,
    show_internal: bool,
    allow_virtual: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetParams {
    target: String,
    #[serde(default)]
    show_internal: bool,
    #[serde(default)]
    allow_virtual: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct DiagnoseParams {
    include_serials: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct NameParams {
    code: Option<String>,
    name: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct ConfigParams {
    key: Option<String>,
    value: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct WatchParams {
    interval_ms: Option<u64>,
    all: bool,
    show_internal: bool,
    allow_virtual: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct TransferParams {
    target: String,
    source: Option<PathBuf>,
    /// La cible est un fichier image (créé avec `size` s'il n'existe pas).
    simulate: bool,
    size: Option<u64>,
    yes: bool,
    dry_run: bool,
    no_verify: bool,
    no_repair_gpt: bool,
    show_internal: bool,
    allow_virtual: bool,
    /// `metadata` ou `data`.
    trace_io: Option<String>,
}

fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, Failure> {
    let value = if params.is_null() {
        json!({})
    } else {
        params.clone()
    };
    serde_json::from_value(value)
        .map_err(|e| failure(Code::ProtocolError, format!("paramètres invalides : {e}")))
}

impl TransferParams {
    fn options(&self, view: &ViewDefaults) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal || view.show_internal,
            allow_virtual: self.allow_virtual || view.allow_virtual,
        }
    }

    fn target(&self) -> Result<Target, Failure> {
        if self.target.is_empty() {
            return Err(failure(Code::ProtocolError, "paramètre « target » requis"));
        }
        Ok(if self.simulate {
            Target::Image {
                path: PathBuf::from(&self.target),
                size: self.size,
            }
        } else {
            Target::Disk(self.target.clone())
        })
    }

    fn trace(&self) -> Result<Option<TraceIo>, Failure> {
        match self.trace_io.as_deref() {
            None => Ok(None),
            Some("metadata") => Ok(Some(TraceIo::Metadata)),
            Some("data") => Ok(Some(TraceIo::Data)),
            Some(autre) => Err(failure(
                Code::ProtocolError,
                format!("trace_io inconnu : « {autre} » (metadata ou data)"),
            )),
        }
    }

    fn source(&self) -> Result<PathBuf, Failure> {
        self.source
            .clone()
            .ok_or_else(|| failure(Code::ProtocolError, "paramètre « source » requis"))
    }
}

// ---------------------------------------------------------------------------------------------
// Méthodes immédiates
// ---------------------------------------------------------------------------------------------

fn enumerate(cfg: &Config) -> Result<Vec<Disk>, Failure> {
    (cfg.enumerate)().map_err(|e| failure(Code::of_enumeration(&e), e))
}

fn json_error(e: serde_json::Error) -> Failure {
    failure(Code::OutputFailed, e)
}

fn quick(method: &str, params: &Value, cfg: &Config) -> Result<Value, Failure> {
    match method {
        "ping" => Ok(json!({"pong": true})),
        "version" => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": PROTOCOL,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        })),
        "methods" => Ok(Value::Array(
            METHODS
                .iter()
                .map(|(name, about)| json!({"method": name, "about": about}))
                .collect(),
        )),
        "disk.list" => {
            let p: ViewParams = parse(params)?;
            let options = FilterOptions {
                show_internal: p.show_internal || cfg.view.show_internal,
                allow_virtual: p.allow_virtual || cfg.view.allow_virtual,
            };
            list::report_value(options, p.all || cfg.view.show_all, &enumerate(cfg)?)
                .map_err(json_error)
        }
        "disk.info" => {
            let p: TargetParams = parse(params)?;
            let options = FilterOptions {
                show_internal: p.show_internal || cfg.view.show_internal,
                allow_virtual: p.allow_virtual || cfg.view.allow_virtual,
            };
            info::report_value(&p.target, options, &enumerate(cfg)?)
                .ok_or_else(|| {
                    failure(
                        Code::DiskNotFound,
                        format!("disque introuvable : « {} »", p.target),
                    )
                })?
                .map_err(json_error)
        }
        "debug.filter" => {
            let p: TargetParams = parse(params)?;
            let options = FilterOptions {
                show_internal: p.show_internal,
                allow_virtual: p.allow_virtual,
            };
            debug::filter_value(&p.target, options, &enumerate(cfg)?)
                .ok_or_else(|| {
                    failure(
                        Code::DiskNotFound,
                        format!("disque introuvable : « {} »", p.target),
                    )
                })?
                .map_err(json_error)
        }
        "diagnose" => {
            let p: DiagnoseParams = parse(params)?;
            diagnose::report_value(
                p.include_serials,
                &diagnose::Environment::system(),
                &(cfg.enumerate)(),
            )
            .map_err(json_error)
        }
        "explain" => {
            let p: NameParams = parse(params)?;
            errors::explain_value(p.code.as_deref()).ok_or_else(|| {
                failure(
                    Code::InvalidArgument,
                    errors::unknown_code_message(p.code.as_deref().unwrap_or_default()),
                )
            })
        }
        "schema" => {
            let p: NameParams = parse(params)?;
            match p.name {
                None => Ok(json!({"schemas": schema::names()})),
                Some(name) => schema::value(&name)
                    .ok_or_else(|| failure(Code::InvalidArgument, schema::unknown_message(&name))),
            }
        }
        "config.list" | "config.get" | "config.set" | "config.reset" => {
            config_method(method, params, cfg)
        }
        _ => Err(failure(
            Code::ProtocolError,
            format!("méthode inconnue : « {method} » (voir la méthode « methods »)"),
        )),
    }
}

fn config_method(method: &str, params: &Value, cfg: &Config) -> Result<Value, Failure> {
    let p: ConfigParams = parse(params)?;
    let need = |name: &str, value: Option<String>| {
        value.ok_or_else(|| failure(Code::ProtocolError, format!("paramètre « {name} » requis")))
    };
    let action = match method {
        "config.list" => ConfigAction::List { json: true },
        "config.get" => ConfigAction::Get {
            key: need("key", p.key)?,
        },
        "config.set" => ConfigAction::Set {
            key: need("key", p.key)?,
            value: need("value", p.value)?,
        },
        _ => ConfigAction::Reset { key: p.key },
    };
    let mut out = Vec::new();
    match config::run(&action, cfg.config_path.as_deref(), &cfg.vars, &mut out) {
        Ok(()) => {
            let text = String::from_utf8_lossy(&out).into_owned();
            if method == "config.list" {
                serde_json::from_str(&text).map_err(json_error)
            } else {
                Ok(json!({"output": text.trim_end()}))
            }
        }
        Err(ConfigError::Invalid(m)) => Err(failure(Code::ConfigInvalid, m)),
        Err(ConfigError::Output(e)) => Err(failure(Code::OutputFailed, e)),
    }
}

// ---------------------------------------------------------------------------------------------
// Opérations longues et surveillance
// ---------------------------------------------------------------------------------------------

struct JobHandle {
    cancel: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct WatchHandle {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
struct State {
    /// Emplacement de l'opération longue : rendu avant la réponse (`slot.rs`), alors que
    /// `thread.is_finished()` ne devient vrai qu'après et ferait refuser à tort la requête suivante.
    slot: JobSlot,
    job: Option<JobHandle>,
    watch: Option<WatchHandle>,
}

#[derive(Clone, Copy)]
enum Kind {
    Write,
    Verify,
    Repair,
}

fn progress_event(p: &Progress, request: &Value) -> Value {
    let mut value = serde_json::to_value(p).unwrap_or_default();
    if let Value::Object(map) = &mut value {
        map.insert("event".into(), "progress".into());
        map.insert("schema_version".into(), 1.into());
        map.insert("request_id".into(), request.clone());
    }
    value
}

fn start_job(
    kind: Kind,
    id: &Value,
    params: &Value,
    cfg: &Config,
    out: &Out,
    state: &mut State,
) -> Result<(), Failure> {
    let Some(guard) = state.slot.try_acquire() else {
        return Err(failure(
            Code::ProtocolError,
            "une opération d'écriture est déjà en cours (voir « cancel »)",
        ));
    };
    // Les paramètres sont validés tout de suite : une erreur de requête répond sans démarrer.
    let p: TransferParams = parse(params)?;
    let (target, options, trace) = (p.target()?, p.options(&cfg.view), p.trace()?);
    let source = match kind {
        Kind::Repair => PathBuf::new(),
        _ => p.source()?,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let (enumerate_fn, make_io) = (Arc::clone(&cfg.enumerate), Arc::clone(&cfg.make_io));
    let (out, id, flag) = (out.clone(), id.clone(), Arc::clone(&cancel));
    out.send(&json!({"event": "job_started", "request_id": id}));
    let thread = std::thread::spawn(move || {
        let io = make_io(flag);
        let enumerate = || enumerate_fn();
        let deps = Deps {
            enumerate: &enumerate,
            io: &io,
        };
        let mut progress = |p: &Progress| {
            out.send(&progress_event(p, &id));
            Control::Continue
        };
        let outcome: Result<Value, job::JobError> = match kind {
            Kind::Write => job::write(
                &WriteJob {
                    target,
                    source,
                    options,
                    verify: !p.no_verify,
                    repair_gpt: !p.no_repair_gpt,
                    trace_io: trace,
                    dry_run: p.dry_run,
                    yes: p.yes,
                },
                &deps,
                &mut progress,
            )
            .and_then(|o| {
                serde_json::to_value(o).map_err(|e| job::JobError {
                    code: Code::OutputFailed,
                    message: e.to_string(),
                })
            }),
            Kind::Verify => job::verify(
                &VerifyJob {
                    target,
                    source,
                    options,
                    trace_io: trace,
                },
                &deps,
                &mut progress,
            )
            .and_then(|o| {
                serde_json::to_value(o).map_err(|e| job::JobError {
                    code: Code::OutputFailed,
                    message: e.to_string(),
                })
            }),
            Kind::Repair => job::repair(
                &RepairJob {
                    target,
                    options,
                    trace_io: trace,
                    dry_run: p.dry_run,
                    yes: p.yes,
                },
                &deps,
            )
            .and_then(|o| {
                serde_json::to_value(o).map_err(|e| job::JobError {
                    code: Code::OutputFailed,
                    message: e.to_string(),
                })
            }),
        };
        guard.release_then(|| {
            out.respond(
                &id,
                outcome.map_err(|e| Failure {
                    code: e.code,
                    message: e.message,
                }),
            );
        });
    });
    state.job = Some(JobHandle { cancel, thread });
    Ok(())
}

fn start_watch(
    params: &Value,
    cfg: &Config,
    out: &Out,
    state: &mut State,
) -> Result<Value, Failure> {
    let p: WatchParams = parse(params)?;
    stop_watch(state);
    let interval = Duration::from_millis(p.interval_ms.unwrap_or(2000).max(100));
    let options = FilterOptions {
        show_internal: p.show_internal || cfg.view.show_internal,
        allow_virtual: p.allow_virtual || cfg.view.allow_virtual,
    };
    let all = p.all || cfg.view.show_all;
    let (enumerate_fn, out) = (Arc::clone(&cfg.enumerate), out.clone());
    let stop = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        let mut last: Option<Value> = None;
        while !flag.load(Ordering::SeqCst) {
            let current = match enumerate_fn() {
                Ok(disks) => list::report_value(options, all, &disks).unwrap_or_default(),
                Err(e) => {
                    json!({"error": {"code": Code::of_enumeration(&e).id(), "message": e.to_string()}})
                }
            };
            if last.as_ref() != Some(&current) {
                out.send(&json!({"event": "disks_changed", "initial": last.is_none(), "report": current}));
                last = Some(current);
            }
            // Attente par petits pas : l'arrêt est pris en compte sans attendre tout l'intervalle.
            let mut waited = Duration::ZERO;
            while waited < interval && !flag.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(20));
                waited += Duration::from_millis(20);
            }
        }
    });
    state.watch = Some(WatchHandle { stop, thread });
    Ok(
        json!({"watching": true, "interval_ms": u64::try_from(interval.as_millis()).unwrap_or(u64::MAX)}),
    )
}

fn stop_watch(state: &mut State) -> bool {
    match state.watch.take() {
        Some(handle) => {
            handle.stop.store(true, Ordering::SeqCst);
            let _ = handle.thread.join();
            true
        }
        None => false,
    }
}

fn handle(line: &str, cfg: &Config, out: &Out, state: &mut State) {
    let request: Request = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(e) => {
            out.respond(
                &Value::Null,
                Err(failure(
                    Code::ProtocolError,
                    format!("requête illisible : {e}"),
                )),
            );
            return;
        }
    };
    log::debug!(target: "bonk::cli", method = request.method.as_str(); "service request");
    let id = &request.id;
    match request.method.as_str() {
        "disk.write" => start_or_fail(Kind::Write, &request, cfg, out, state),
        "disk.verify" => start_or_fail(Kind::Verify, &request, cfg, out, state),
        "disk.repair_gpt" => start_or_fail(Kind::Repair, &request, cfg, out, state),
        "cancel" => {
            let running = state.slot.is_busy();
            if let (true, Some(job)) = (running, &state.job) {
                job.cancel.store(true, Ordering::SeqCst);
            }
            out.respond(id, Ok(json!({"cancelling": running})));
        }
        "watch.start" => out.respond(id, start_watch(&request.params, cfg, out, state)),
        "watch.stop" => out.respond(
            id,
            Ok(json!({"watching": false, "was_watching": stop_watch(state)})),
        ),
        method => out.respond(id, quick(method, &request.params, cfg)),
    }
}

fn start_or_fail(kind: Kind, request: &Request, cfg: &Config, out: &Out, state: &mut State) {
    if let Err(f) = start_job(kind, &request.id, &request.params, cfg, out, state) {
        out.respond(&request.id, Err(f));
    }
}

/// Boucle du service : lit les requêtes jusqu'à la fin de l'entrée, puis annule l'opération en cours
/// et s'arrête. Rend le code de sortie.
pub(crate) fn serve(input: impl BufRead, output: Box<dyn Write + Send>, cfg: &Config) -> u8 {
    let out = Out(Arc::new(Mutex::new(output)));
    out.send(&json!({"event": "ready", "protocol": PROTOCOL, "version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS}));
    let mut state = State::default();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        handle(&line, cfg, &out, &mut state);
    }
    // Fin de l'entrée : le client est parti, on ne laisse rien tourner derrière lui.
    if let Some(job) = state.job.take() {
        job.cancel.store(true, Ordering::SeqCst);
        let _ = job.thread.join();
    }
    stop_watch(&mut state);
    0
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use std::io::{self, Cursor, Read};
    use std::sync::atomic::AtomicUsize;

    use bonk_core::{BlockDevice, BlockError, MemoryDevice};
    use bonk_hal::{Bus, DiskKind, Partition};

    use super::*;

    const DISK_SIZE: usize = 8 << 20;

    /// Périphérique en mémoire partagé entre fils ; `slow` ralentit les écritures pour qu'une
    /// opération reste en cours le temps d'envoyer une annulation.
    #[derive(Clone)]
    struct Shared {
        memory: Arc<Mutex<MemoryDevice>>,
        slow: bool,
    }

    impl BlockDevice for Shared {
        fn size(&self) -> u64 {
            self.memory.lock().unwrap().size()
        }
        fn block_size(&self) -> u32 {
            512
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            self.memory.lock().unwrap().read_at(offset, buf)
        }
        fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
            if self.slow {
                std::thread::sleep(Duration::from_millis(60));
            }
            self.memory.lock().unwrap().write_at(offset, data)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }

    fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
        Partition {
            path: path.to_owned(),
            label: None,
            fs_type: Some("vfat".to_owned()),
            size_bytes: 1 << 20,
            mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
            holders: vec![],
            is_system_volume: system,
            is_active_esp: false,
        }
    }

    fn usb(id: &str) -> Disk {
        Disk {
            id: id.to_owned(),
            path: format!("/dev/{id}"),
            model: "Cle".to_owned(),
            serial: Some("S1".to_owned()),
            size_bytes: DISK_SIZE as u64,
            bus: Bus::Usb,
            kind: DiskKind::Physical,
            removable_flag: true,
            fs_type: None,
            holders: vec![],
            partitions: vec![partition("/dev/x1", &[], false)],
        }
    }

    fn system() -> Disk {
        Disk {
            bus: Bus::Nvme,
            partitions: vec![partition("/dev/nvme0n1p2", &["/"], true)],
            ..usb("nvme-sys")
        }
    }

    struct Fake {
        device: Shared,
        cfg: Config,
    }

    fn fake(slow: bool, enumerate: EnumerateFn) -> Fake {
        let device = Shared {
            memory: Arc::new(Mutex::new(
                MemoryDevice::new(DISK_SIZE as u64, 512).unwrap(),
            )),
            slow,
        };
        let dev = device.clone();
        Fake {
            device,
            cfg: Config {
                enumerate,
                make_io: Arc::new(move |cancel| {
                    let dev = dev.clone();
                    Io {
                        unmount: Box::new(|_| Ok(vec![])),
                        open_node: Box::new(move |_, _| {
                            Ok(Box::new(dev.clone()) as Box<dyn BlockDevice>)
                        }),
                        cancelled: Box::new(move || cancel.load(Ordering::SeqCst)),
                    }
                }),
                config_path: None,
                vars: vec![],
                view: ViewDefaults::default(),
            },
        }
    }

    fn fixed() -> EnumerateFn {
        Arc::new(|| Ok(vec![usb("usb-a"), system()]))
    }

    #[derive(Clone)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Entrée qui attend avant de se fermer : laisse le temps aux événements asynchrones.
    struct SlowEof {
        inner: Cursor<Vec<u8>>,
        wait: Duration,
    }

    impl Read for SlowEof {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            if n == 0 {
                std::thread::sleep(self.wait);
            }
            Ok(n)
        }
    }

    impl BufRead for SlowEof {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            let empty = self.inner.fill_buf()?.is_empty();
            if empty {
                std::thread::sleep(self.wait);
                return Ok(&[]);
            }
            self.inner.fill_buf()
        }
        fn consume(&mut self, amt: usize) {
            self.inner.consume(amt);
        }
    }

    /// Entrée qui ne se ferme qu'une fois qu'un message attendu est sorti : une opération longue a
    /// le temps de finir, sans durée fixe à deviner.
    struct UntilSeen {
        inner: Cursor<Vec<u8>>,
        output: Buffer,
        needle: String,
    }

    impl Read for UntilSeen {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl BufRead for UntilSeen {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.inner.fill_buf()?.is_empty() {
                let limit = std::time::Instant::now() + Duration::from_secs(20);
                while std::time::Instant::now() < limit {
                    let text = String::from_utf8_lossy(&self.output.0.lock().unwrap()).into_owned();
                    if text.contains(&self.needle) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                return Ok(&[]);
            }
            self.inner.fill_buf()
        }
        fn consume(&mut self, amt: usize) {
            self.inner.consume(amt);
        }
    }

    /// Comme [`session`], mais attend `needle` dans la sortie avant de fermer l'entrée.
    fn session_until(requests: &[&str], cfg: &Config, needle: &str) -> Vec<Value> {
        let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
        let input = UntilSeen {
            inner: Cursor::new((requests.join("\n") + "\n").into_bytes()),
            output: buffer.clone(),
            needle: needle.to_owned(),
        };
        assert_eq!(serve(input, Box::new(buffer.clone()), cfg), 0);
        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
            .collect()
    }

    fn session(requests: &[&str], cfg: &Config, wait: Duration) -> Vec<Value> {
        let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
        let input = SlowEof {
            inner: Cursor::new((requests.join("\n") + "\n").into_bytes()),
            wait,
        };
        let code = serve(input, Box::new(buffer.clone()), cfg);
        assert_eq!(code, 0);
        let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
        text.lines()
            .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
            .collect()
    }

    fn quick_session(requests: &[&str]) -> Vec<Value> {
        session(requests, &fake(false, fixed()).cfg, Duration::ZERO)
    }

    fn response(messages: &[Value], id: u64) -> &Value {
        messages
            .iter()
            .find(|m| m["id"] == id)
            .expect("réponse attendue")
    }

    struct TempImage(PathBuf);
    impl TempImage {
        fn new(label: &str, len: usize) -> Self {
            let path =
                std::env::temp_dir().join(format!("bonk-serve-{}-{label}.img", std::process::id()));
            std::fs::write(
                &path,
                (0..len).map(|i| (i * 11 % 251) as u8).collect::<Vec<u8>>(),
            )
            .unwrap();
            Self(path)
        }
    }
    impl Drop for TempImage {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn demarrage_ping_version_methodes() {
        let out = quick_session(&[
            r#"{"id":1,"method":"ping"}"#,
            r#"{"id":2,"method":"version"}"#,
            r#"{"id":3,"method":"methods"}"#,
        ]);
        assert_eq!(out[0]["event"], "ready");
        assert_eq!(out[0]["protocol"], PROTOCOL);
        assert_eq!(response(&out, 1)["result"]["pong"], true);
        assert_eq!(response(&out, 2)["result"]["protocol"], PROTOCOL);
        let methods = response(&out, 3)["result"].as_array().unwrap().clone();
        assert_eq!(methods.len(), METHODS.len());
        assert!(methods.iter().any(|m| m["method"] == "disk.write"));
    }

    #[test]
    fn l_id_est_rendu_tel_quel_quel_que_soit_son_type() {
        let out = quick_session(&[
            r#"{"id":"abc","method":"ping"}"#,
            r#"{"id":{"x":[1]},"method":"ping"}"#,
            r#"{"method":"ping"}"#,
        ]);
        assert_eq!(out[1]["id"], "abc");
        assert_eq!(out[2]["id"], json!({"x": [1]}));
        assert_eq!(out[3]["id"], Value::Null);
    }

    #[test]
    fn disk_list_meme_contenu_que_la_cli() {
        let out = quick_session(&[
            r#"{"id":1,"method":"disk.list"}"#,
            r#"{"id":2,"method":"disk.list","params":{"all":true}}"#,
        ]);
        let default = &response(&out, 1)["result"];
        assert_eq!(default["schema_version"], 1);
        assert_eq!(default["disks"].as_array().unwrap().len(), 1);
        assert_eq!(default["disks"][0]["id"], "usb-a");
        let all = &response(&out, 2)["result"];
        assert_eq!(all["disks"].as_array().unwrap().len(), 2);
        assert_eq!(all["disks"][1]["eligibility"]["verdict"], "excluded");
        let expected =
            list::report_value(FilterOptions::default(), true, &[usb("usb-a"), system()]).unwrap();
        assert_eq!(*all, expected);
    }

    #[test]
    fn info_et_debug_filter() {
        let out = quick_session(&[
            r#"{"id":1,"method":"disk.info","params":{"target":"usb-a"}}"#,
            r#"{"id":2,"method":"disk.info","params":{"target":"absent"}}"#,
            r#"{"id":3,"method":"debug.filter","params":{"target":"nvme-sys"}}"#,
        ]);
        assert_eq!(response(&out, 1)["result"]["disk"]["id"], "usb-a");
        assert_eq!(response(&out, 2)["error"]["code"], "E1003");
        let rules = response(&out, 3)["result"]["rules"]
            .as_array()
            .unwrap()
            .clone();
        assert!(
            rules
                .iter()
                .any(|r| r["rule"] == "system_volume" && r["fired"] == true)
        );
    }

    #[test]
    fn erreurs_de_protocole_avec_leur_code() {
        let out = quick_session(&[
            "ceci n'est pas du json",
            r#"{"id":1,"method":"inconnue"}"#,
            r#"{"id":2,"method":"disk.list","params":{"nimporte":true}}"#,
            r#"{"id":3,"method":"disk.info","params":{}}"#,
            r#"{"id":4,"method":"config.set","params":{"key":"lang"}}"#,
        ]);
        assert_eq!(out[1]["id"], Value::Null, "{out:?}");
        assert_eq!(out[1]["error"]["code"], "E6001");
        for id in 1..=4 {
            let error = &response(&out, id)["error"];
            assert_eq!(error["code"], "E6001", "{id} : {error}");
            assert_eq!(error["exit_code"], 1);
        }
        assert!(
            response(&out, 1)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("methods")
        );
    }

    #[test]
    fn explain_schema_et_configuration() {
        let file =
            std::env::temp_dir().join(format!("bonk-serve-config-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&file);
        let mut fake = fake(false, fixed());
        fake.cfg.config_path = Some(file.clone());
        let out = session(
            &[
                r#"{"id":1,"method":"explain","params":{"code":"E4002"}}"#,
                r#"{"id":2,"method":"explain","params":{"code":"E0"}}"#,
                r#"{"id":3,"method":"schema","params":{"name":"write"}}"#,
                r#"{"id":4,"method":"schema"}"#,
                r#"{"id":5,"method":"config.set","params":{"key":"view.show_all","value":"oui"}}"#,
                r#"{"id":6,"method":"config.get","params":{"key":"view.show_all"}}"#,
                r#"{"id":7,"method":"config.list"}"#,
                r#"{"id":8,"method":"config.set","params":{"key":"lang","value":"en"}}"#,
                r#"{"id":9,"method":"disk.list"}"#,
            ],
            &fake.cfg,
            Duration::ZERO,
        );
        assert_eq!(response(&out, 1)["result"]["codes"][0]["exit_code"], 4);
        assert_eq!(response(&out, 2)["error"]["code"], "E0001");
        assert_eq!(response(&out, 3)["result"]["type"], "object");
        assert!(
            response(&out, 4)["result"]["schemas"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n == "serve-request")
        );
        assert_eq!(response(&out, 6)["result"]["output"], "true");
        assert_eq!(response(&out, 7)["result"]["settings"][3]["value"], "true");
        assert_eq!(response(&out, 8)["error"]["code"], "E5002");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn ecriture_avec_progression_puis_resultat() {
        let source = TempImage::new("ecriture", 9 << 20);
        let Fake { device, cfg } = fake(false, fixed());
        // Image de 9 Mio sur un disque de 8 Mio : trop grande ; on en prend une qui tient.
        let small = TempImage::new("petite", 6 << 20);
        let request = format!(
            r#"{{"id":7,"method":"disk.write","params":{{"target":"usb-a","source":{:?},"yes":true}}}}"#,
            small.0.display().to_string()
        );
        let too_big = format!(
            r#"{{"id":8,"method":"disk.write","params":{{"target":"usb-a","source":{:?},"yes":true}}}}"#,
            source.0.display().to_string()
        );
        let out = session_until(&[&request], &cfg, r#""id":7"#);
        let result = &response(&out, 7)["result"];
        assert_eq!(result["event"], "result", "{out:?}");
        assert_eq!(result["write"]["bytes"], 6 << 20);
        assert!(result["verify"]["bytes"] == 6 << 20);
        // L'ordre des messages : prêt, démarrage, progression (avec l'id de la requête), résultat.
        assert_eq!(out[1]["event"], "job_started");
        assert_eq!(out[1]["request_id"], 7);
        let progress: Vec<&Value> = out.iter().filter(|m| m["event"] == "progress").collect();
        assert!(progress.len() >= 4 && progress.iter().all(|p| p["request_id"] == 7));
        assert_eq!(out.last().unwrap()["id"], 7);
        let written = device.memory.lock().unwrap().as_bytes()[..6 << 20].to_vec();
        assert_eq!(written, std::fs::read(&small.0).unwrap());
        // Trop grande : refusée avec son code, sans toucher au disque.
        let refused = session(&[&too_big], &fake(false, fixed()).cfg, Duration::ZERO);
        assert_eq!(response(&refused, 8)["error"]["code"], "E3003");
    }

    #[test]
    fn les_garde_fous_de_la_cli_s_appliquent_au_service() {
        let source = TempImage::new("garde", 1 << 20);
        let path = source.0.display().to_string();
        let sans_yes = format!(
            r#"{{"id":1,"method":"disk.write","params":{{"target":"usb-a","source":{path:?}}}}}"#
        );
        let interdit = format!(
            r#"{{"id":2,"method":"disk.write","params":{{"target":"nvme-sys","source":{path:?},"yes":true,"show_internal":true}}}}"#
        );
        let plan = format!(
            r#"{{"id":3,"method":"disk.write","params":{{"target":"usb-a","source":{path:?},"dry_run":true}}}}"#
        );
        let par_chemin = format!(
            r#"{{"id":4,"method":"disk.write","params":{{"target":"/dev/usb-a","source":{path:?},"yes":true}}}}"#
        );
        let sans_source =
            r#"{"id":5,"method":"disk.write","params":{"target":"usb-a","yes":true}}"#.to_owned();
        let f = fake(false, fixed());
        // Les opérations s'enchaînent : une à la fois, chacune dans sa session.
        for (id, request, code) in [
            (1, sans_yes, "E2004"),
            (2, interdit, "E2001"),
            (4, par_chemin, "E1003"),
            (5, sans_source, "E6001"),
        ] {
            let out = session(&[&request], &f.cfg, Duration::ZERO);
            assert_eq!(response(&out, id)["error"]["code"], code, "{id}");
        }
        let out = session(&[&plan], &f.cfg, Duration::ZERO);
        assert_eq!(response(&out, 3)["result"]["event"], "plan");
        // Rien n'a été écrit par aucune de ces requêtes.
        assert!(
            f.device
                .memory
                .lock()
                .unwrap()
                .as_bytes()
                .iter()
                .all(|b| *b == 0)
        );
    }

    #[test]
    fn annulation_et_une_seule_operation_a_la_fois() {
        let source = TempImage::new("annule", 7 << 20);
        let path = source.0.display().to_string();
        let write = |id: u64| {
            format!(
                r#"{{"id":{id},"method":"disk.write","params":{{"target":"usb-a","source":{path:?},"yes":true,"no_verify":true}}}}"#
            )
        };
        let Fake { device, cfg } = fake(true, fixed());
        let out = session(
            &[&write(1), &write(2), r#"{"id":3,"method":"cancel"}"#],
            &cfg,
            Duration::from_millis(0),
        );
        // La seconde opération est refusée pendant que la première tourne.
        assert_eq!(response(&out, 2)["error"]["code"], "E6001");
        assert!(
            response(&out, 2)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("déjà en cours")
        );
        assert_eq!(response(&out, 3)["result"]["cancelling"], true);
        // La première est annulée proprement : code dédié, support partiel.
        let first = response(&out, 1);
        assert_eq!(first["error"]["code"], "E4003", "{first}");
        assert_eq!(first["error"]["exit_code"], 5);
        let written = device
            .memory
            .lock()
            .unwrap()
            .as_bytes()
            .iter()
            .filter(|b| **b != 0)
            .count();
        assert!(written < 7 << 20, "l'écriture aurait dû s'arrêter en route");
        // Sans opération en cours, cancel répond qu'il n'y a rien à annuler.
        let idle = quick_session(&[r#"{"id":1,"method":"cancel"}"#]);
        assert_eq!(response(&idle, 1)["result"]["cancelling"], false);
    }

    #[test]
    fn fermer_l_entree_annule_l_operation_et_rend_la_main() {
        let source = TempImage::new("eof", 7 << 20);
        let path = source.0.display().to_string();
        let write = format!(
            r#"{{"id":1,"method":"disk.write","params":{{"target":"usb-a","source":{path:?},"yes":true,"no_verify":true}}}}"#
        );
        let f = fake(true, fixed());
        let started = std::time::Instant::now();
        let out = session(&[&write], &f.cfg, Duration::ZERO);
        // La session se termine sans attendre les ~7 blocs lents, et l'opération est dite annulée.
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(response(&out, 1)["error"]["code"], "E4003");
    }

    #[test]
    fn surveillance_des_disques() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let enumerate: EnumerateFn = Arc::new(move || {
            // La troisième énumération voit une seconde clé.
            if counter.fetch_add(1, Ordering::SeqCst) >= 2 {
                Ok(vec![usb("usb-a"), usb("usb-b")])
            } else {
                Ok(vec![usb("usb-a")])
            }
        });
        let f = fake(false, enumerate);
        // On attend le second événement au lieu de deviner une durée : sur un exécuteur chargé (macOS en CI),
        // l'intervalle de 100 ms s'allonge et une attente fixe de 700 ms ne voyait que l'événement initial.
        let out = session_until(
            &[r#"{"id":1,"method":"watch.start","params":{"interval_ms":100}}"#],
            &f.cfg,
            r#""initial":false"#,
        );
        assert_eq!(response(&out, 1)["result"]["watching"], true);
        let events: Vec<&Value> = out
            .iter()
            .filter(|m| m["event"] == "disks_changed")
            .collect();
        assert_eq!(
            events.len(),
            2,
            "un événement initial puis un changement : {events:?}"
        );
        assert_eq!(events[0]["initial"], true);
        assert_eq!(events[0]["report"]["disks"].as_array().unwrap().len(), 1);
        assert_eq!(events[1]["initial"], false);
        assert_eq!(events[1]["report"]["disks"].as_array().unwrap().len(), 2);
        // watch.stop arrête la surveillance.
        let f = fake(false, fixed());
        let out = session(
            &[
                r#"{"id":1,"method":"watch.start","params":{"interval_ms":100}}"#,
                r#"{"id":2,"method":"watch.stop"}"#,
            ],
            &f.cfg,
            Duration::from_millis(300),
        );
        assert_eq!(response(&out, 2)["result"]["was_watching"], true);
        // Arrêtée aussitôt, la surveillance a émis au plus l'événement initial, et plus rien ensuite.
        assert!(out.iter().filter(|m| m["event"] == "disks_changed").count() <= 1);
    }

    #[test]
    fn echec_d_enumeration_rapporte_avec_son_code() {
        let broken: EnumerateFn = Arc::new(|| Err(EnumerateError::Unsupported { os: "test" }));
        let out = session(
            &[
                r#"{"id":1,"method":"disk.list"}"#,
                r#"{"id":2,"method":"diagnose"}"#,
            ],
            &fake(false, broken).cfg,
            Duration::ZERO,
        );
        assert_eq!(response(&out, 1)["error"]["code"], "E1002");
        // `diagnose` rapporte l'échec dans son rapport au lieu d'échouer.
        assert_eq!(response(&out, 2)["result"]["enumeration"]["ok"], false);
    }
}
