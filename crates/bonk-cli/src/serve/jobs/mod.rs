use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::thread::JoinHandle;

use bonk_core::write::{Control, Progress};
use serde::Serialize;
use serde_json::{Value, json};

use crate::errors::Code;
use crate::fetch;
use bonk_ops::job::{
    self, Deps, FormatJob, RepairJob, VaultAddJob, VaultInitJob, VaultListJob, VaultRemoveJob,
    VaultUpdateJob, VerifyJob, WriteJob,
};

use super::params::TransferParams;
use super::protocol::{Failure, Out, Request, failure, parse};
use super::{Config, State};

mod disk;
mod vault;
mod watch;

pub(in crate::serve) use disk::*;
pub(in crate::serve) use vault::*;
pub(in crate::serve) use watch::*;

pub(super) struct JobHandle {
    pub(super) cancel: Arc<AtomicBool>,
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

fn required<T>(value: Option<T>, name: &str) -> Result<T, Failure> {
    value.ok_or_else(|| failure(Code::ProtocolError, format!("parameter '{name}' required")))
}
