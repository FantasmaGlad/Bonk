use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use bonk_core::format::Filesystem;
use bonk_core::format::table::TableKind;
use bonk_core::write::{Control, Progress};
use bonk_hal::{Elevation, FilterOptions};
use serde::Serialize;
use serde_json::{Value, json};

use crate::errors::Code;
use crate::fetch;
use crate::list;
use bonk_ops::job::{
    self, Deps, FormatJob, RepairJob, Target, TraceIo, VaultAddJob, VaultInitJob, VaultListJob,
    VaultRemoveJob, VaultUpdateJob, VerifyJob, WriteJob, WriteMode,
};

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
pub(super) enum DiskKind {
    Write,
    Verify,
    Repair,
    Format,
}

#[derive(Clone, Copy)]
pub(super) enum VaultKind {
    List,
    Init,
    Add,
    Remove,
    Update,
}

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Disk(DiskKind),
    Vault(VaultKind),
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

pub(super) enum Task {
    Write(Box<WriteJob>, Option<String>, Option<PathBuf>),
    Verify(VerifyJob),
    Repair(RepairJob),
    Format(FormatJob),
    VaultList(VaultListJob),
    VaultInit(Box<VaultInitJob>),
    VaultAdd(VaultAddJob),
    VaultRemove(VaultRemoveJob),
    VaultUpdate(Box<VaultUpdateJob>),
}

/// A parameter error answers without starting anything.
pub(super) fn prepare(kind: Kind, params: &Value, cfg: &Config) -> Result<Task, Failure> {
    let p: TransferParams = parse(params)?;
    match kind {
        Kind::Disk(kind) => prepare_disk(kind, p, cfg),
        Kind::Vault(kind) => prepare_vault(kind, &p, cfg),
    }
}

/// Every parameter is parsed whatever the operation, so a malformed one is always refused.
struct DiskArgs {
    options: FilterOptions,
    target: Target,
    trace: Option<TraceIo>,
    mode: WriteMode,
    elevation: Elevation,
    filesystem: Option<Filesystem>,
    table: TableKind,
}

fn disk_args(kind: DiskKind, p: &TransferParams, cfg: &Config) -> Result<DiskArgs, Failure> {
    let options = match kind {
        DiskKind::Verify => p.options(&cfg.view),
        DiskKind::Write | DiskKind::Repair | DiskKind::Format => p.write_options(),
    };
    let default_table = match kind {
        DiskKind::Format => TableKind::FORMAT_DEFAULT,
        DiskKind::Write | DiskKind::Verify | DiskKind::Repair => TableKind::WINDOWS_DEFAULT,
    };
    Ok(DiskArgs {
        options,
        target: p.target()?,
        trace: p.trace()?,
        mode: p.mode()?,
        elevation: p.elevation()?,
        filesystem: p.filesystem()?,
        table: p.table(default_table)?,
    })
}

fn prepare_disk(kind: DiskKind, p: TransferParams, cfg: &Config) -> Result<Task, Failure> {
    let a = disk_args(kind, &p, cfg)?;
    Ok(match kind {
        DiskKind::Write => write_task(a, p)?,
        DiskKind::Verify => Task::Verify(VerifyJob {
            target: a.target,
            source: p.source()?,
            options: a.options,
            trace_io: a.trace,
            elevation: a.elevation,
        }),
        DiskKind::Repair => Task::Repair(RepairJob {
            target: a.target,
            options: a.options,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            elevation: a.elevation,
            force: p.force,
        }),
        DiskKind::Format => Task::Format(FormatJob {
            target: a.target,
            filesystem: a.filesystem,
            table: a.table,
            label: p.label,
            options: a.options,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            force: p.force,
            elevation: a.elevation,
        }),
    })
}

/// A catalog image takes its hash from the signed catalog; no client hash may replace it.
fn write_source(p: &TransferParams) -> Result<(PathBuf, Option<String>), Failure> {
    let Some(id) = &p.image else {
        return Ok((p.source()?, p.sha256.clone()));
    };
    if p.sha256.is_some() || p.sha256_file.is_some() {
        return Err(failure(
            Code::InvalidArgument,
            "image takes its hash from the signed catalog: sha256 and sha256_file are refused",
        ));
    }
    let (source, hash) =
        crate::catalog::image_source(id, p.catalog_url.as_deref(), p.cache_dir.as_deref())
            .map_err(|e| failure(e.code, e.message))?;
    Ok((source, Some(hash)))
}

fn write_task(a: DiskArgs, p: TransferParams) -> Result<Task, Failure> {
    let (source, expected_sha256) = write_source(&p)?;
    Ok(Task::Write(
        Box::new(WriteJob {
            target: a.target,
            source,
            options: a.options,
            verify: !p.no_verify,
            repair_gpt: !p.no_repair_gpt,
            trace_io: a.trace,
            dry_run: p.dry_run,
            yes: p.yes,
            expected_sha256,
            mode: a.mode,
            table: a.table,
            elevation: a.elevation,
            force: p.force,
        }),
        p.sha256_file,
        p.cache_dir,
    ))
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
    // Parameters are validated at once: a request error answers without starting.
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
    let options = cfg.view.widen(FilterOptions {
        show_internal: p.show_internal,
        allow_virtual: p.allow_virtual,
    });
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
            // Small steps: a stop is seen without waiting the whole interval.
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
            if handle.thread.join().is_err() {
                log::error!(target: "bonk::cli", "the disk watch panicked");
            }
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
            Self::VaultList(j) => job::vault_list(&j, deps).and_then(to_json),
            Self::VaultInit(j) => job::vault_init(&j, deps).and_then(to_json),
            Self::VaultAdd(j) => job::vault_add(&j, deps, progress).and_then(to_json),
            Self::VaultRemove(j) => job::vault_remove(&j, deps).and_then(to_json),
            Self::VaultUpdate(j) => job::vault_update(&j, deps).and_then(to_json),
        }
    }
}

/// GRUB of the signed catalog, for `vault.init` and `vault.update`; a refusal answers before anything starts.
fn vault_grub(
    p: &TransferParams,
    cfg: &Config,
) -> Result<(Vec<bonk_core::vault::BootFile>, job::GrubInfo), Failure> {
    let source = crate::vault::GrubSource {
        grub_dir: p.grub_dir.clone(),
        catalog_url: p.catalog_url.clone(),
        cache_dir: p.cache_dir.clone(),
    };
    crate::vault::grub(&source, &cfg.catalog).map_err(|e| failure(e.code, e.message))
}

fn prepare_vault(kind: VaultKind, p: &TransferParams, cfg: &Config) -> Result<Task, Failure> {
    let (target, trace_io, elevation) = (p.target()?, p.trace()?, p.elevation()?);
    Ok(match kind {
        VaultKind::List => Task::VaultList(VaultListJob {
            target,
            options: p.options(&cfg.view),
            trace_io,
            elevation,
        }),
        VaultKind::Init => vault_init_task(target, trace_io, elevation, p, cfg)?,
        VaultKind::Add => Task::VaultAdd(VaultAddJob {
            target,
            source: required(p.iso.clone(), "iso")?,
            name: None,
            options: p.write_options(),
            trace_io,
            yes: p.yes,
            elevation,
        }),
        VaultKind::Remove => Task::VaultRemove(VaultRemoveJob {
            target,
            name: required(p.name.clone(), "name")?,
            options: p.write_options(),
            trace_io,
            yes: p.yes,
            elevation,
        }),
        VaultKind::Update => {
            let (boot, grub) = vault_grub(p, cfg)?;
            Task::VaultUpdate(Box::new(VaultUpdateJob {
                target,
                boot,
                grub: Some(grub),
                options: p.write_options(),
                trace_io,
                dry_run: p.dry_run || !p.yes,
                yes: p.yes,
                elevation,
            }))
        }
    })
}

fn vault_init_task(
    target: Target,
    trace_io: Option<TraceIo>,
    elevation: Elevation,
    p: &TransferParams,
    cfg: &Config,
) -> Result<Task, Failure> {
    // A plan or a refusal for want of confirmation writes nothing: no GRUB needed.
    let (boot, grub) = if p.dry_run || !(p.yes || p.force) {
        (Vec::new(), None)
    } else {
        let (boot, grub) = vault_grub(p, cfg)?;
        (boot, Some(grub))
    };
    Ok(Task::VaultInit(Box::new(VaultInitJob {
        target,
        boot,
        grub,
        table: p.table(TableKind::VAULT_DEFAULT)?,
        options: p.write_options(),
        trace_io,
        dry_run: p.dry_run,
        yes: p.yes,
        force: p.force,
        elevation,
    })))
}

fn required<T>(value: Option<T>, name: &str) -> Result<T, Failure> {
    value.ok_or_else(|| failure(Code::ProtocolError, format!("parameter '{name}' required")))
}
