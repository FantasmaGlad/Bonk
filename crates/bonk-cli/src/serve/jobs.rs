//! Opérations longues et surveillance de la liste des disques, chacune sur son fil.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use bonk_core::write::{Control, Progress};
use bonk_hal::FilterOptions;
use serde::Serialize;
use serde_json::{Value, json};

use crate::errors::Code;
use crate::fetch;
use crate::list;
use bonk_ops::job::{self, Deps, FormatJob, RepairJob, VerifyJob, WriteJob};

use super::params::{TransferParams, WatchParams};
use super::protocol::{Failure, Out, Request, failure, parse};
use super::{Config, State};

pub(super) struct JobHandle {
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) thread: JoinHandle<()>,
}

pub(super) struct WatchHandle {
    pub(super) stop: Arc<AtomicBool>,
    pub(super) thread: JoinHandle<()>,
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Write,
    Verify,
    Repair,
    Format,
}

pub(super) fn to_json(report: impl Serialize) -> Result<Value, job::JobError> {
    serde_json::to_value(report).map_err(|e| job::JobError {
        code: Code::OutputFailed,
        message: e.to_string(),
    })
}

pub(super) fn progress_event(p: &Progress, request: &Value) -> Value {
    let mut value = serde_json::to_value(p).unwrap_or_default();
    if let Value::Object(map) = &mut value {
        map.insert("event".into(), "progress".into());
        map.insert("schema_version".into(), job::SCHEMA_VERSION.into());
        map.insert("request_id".into(), request.clone());
    }
    value
}

/// Une opération longue validée, prête à partir sur son fil.
pub(super) enum Task {
    /// L'écriture, avec le fichier d'empreintes et le dossier de cache d'une image à télécharger.
    Write(Box<WriteJob>, Option<String>, Option<PathBuf>),
    Verify(VerifyJob),
    Repair(RepairJob),
    Format(FormatJob),
}

/// Convertit la requête en tâche typée ; une erreur de paramètre répond sans rien démarrer.
pub(super) fn prepare(kind: Kind, params: &Value, cfg: &Config) -> Result<Task, Failure> {
    let p: TransferParams = parse(params)?;
    let options = match kind {
        Kind::Verify => p.options(&cfg.view),
        Kind::Write | Kind::Repair | Kind::Format => p.write_options(),
    };
    let (target, trace) = (p.target()?, p.trace()?);
    let mode = p.mode()?;
    let elevation = p.elevation()?;
    let (filesystem, table) = (p.filesystem()?, p.table()?);
    let source = match kind {
        Kind::Repair | Kind::Format => PathBuf::new(),
        _ => p.source()?,
    };
    Ok(match kind {
        Kind::Write => Task::Write(
            Box::new(WriteJob {
                target,
                source,
                options,
                verify: !p.no_verify,
                repair_gpt: !p.no_repair_gpt,
                trace_io: trace,
                dry_run: p.dry_run,
                yes: p.yes,
                expected_sha256: p.sha256,
                mode,
                table,
                elevation,
                force: p.force,
            }),
            p.sha256_file,
            p.cache_dir,
        ),
        Kind::Verify => Task::Verify(VerifyJob {
            target,
            source,
            options,
            trace_io: trace,
            elevation,
        }),
        Kind::Repair => Task::Repair(RepairJob {
            target,
            options,
            trace_io: trace,
            dry_run: p.dry_run,
            yes: p.yes,
            elevation,
            force: p.force,
        }),
        Kind::Format => Task::Format(FormatJob {
            target,
            filesystem,
            table,
            label: p.label,
            options,
            trace_io: trace,
            dry_run: p.dry_run,
            yes: p.yes,
            force: p.force,
            elevation,
        }),
    })
}

pub(super) fn start_job(
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
            "a write operation is already running (see 'cancel')",
        ));
    };
    // Les paramètres sont validés tout de suite : une erreur de requête répond sans démarrer.
    let task = prepare(kind, params, cfg)?;
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
        let outcome = task.run(&deps, &mut progress);
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

pub(super) fn start_watch(
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

pub(super) fn stop_watch(state: &mut State) -> bool {
    match state.watch.take() {
        Some(handle) => {
            handle.stop.store(true, Ordering::SeqCst);
            let _ = handle.thread.join();
            true
        }
        None => false,
    }
}

pub(super) fn start_or_fail(
    kind: Kind,
    request: &Request,
    cfg: &Config,
    out: &Out,
    state: &mut State,
) {
    if let Err(f) = start_job(kind, &request.id, &request.params, cfg, out, state) {
        out.respond(&request.id, Err(f));
    }
}

impl Task {
    pub(super) fn run(
        self,
        deps: &Deps<'_>,
        progress: &mut dyn FnMut(&Progress) -> Control,
    ) -> Result<Value, job::JobError> {
        match self {
            Self::Write(mut j, sha256_file, cache_dir) => {
                let source = j.source.clone();
                let request = fetch::Request {
                    source: &source,
                    sha256_file: sha256_file.as_deref(),
                    cache_dir: cache_dir.as_deref(),
                };
                fetch::write(&request, &mut j, deps, progress).and_then(to_json)
            }
            Self::Verify(j) => job::verify(&j, deps, progress).and_then(to_json),
            Self::Repair(j) => job::repair(&j, deps).and_then(to_json),
            Self::Format(j) => job::format(&j, deps).and_then(to_json),
        }
    }
}
