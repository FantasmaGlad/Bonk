//! L'écriture sur le téléphone : le moteur et les contrôles de la ligne de commande, sur la clé qu'Android a
//! confiée à l'application. Seul l'accès au matériel change.

use std::path::Path;
use std::sync::atomic::Ordering;

use bonk_core::format::table::TableKind;
use bonk_core::write::{Control, Progress};
use bonk_core::{BlockDevice, BlockError};
use bonk_hal::{Disk, Elevation, FilterOptions};
use bonk_ops::job::{self, Deps, Io, Target, WriteJob, WriteMode, WriteOutcome};
use bonk_ui::event::{Event, Outcome, parse_line};

use crate::screen::{CANCEL, EVENTS, MODEL};

fn push(event: Event) {
    if let Ok(mut events) = EVENTS.lock() {
        events.push_back(event);
    }
}

fn unavailable(what: &str) -> BlockError {
    BlockError::Io(std::io::Error::other(format!(
        "{what}: not available on Android"
    )))
}

/// `open` ouvre la clé confiée par l'application. Ni démontage ni relecture de table : Android a détaché son
/// pilote en donnant la clé, et la reprendra quand l'application la rendra.
fn android_io(open: impl Fn() -> Result<Box<dyn BlockDevice>, BlockError> + 'static) -> Io {
    Io {
        prepare: Box::new(|_, _, _| Ok(())),
        unmount: Box::new(|_, _| Ok(Vec::new())),
        open_node: Box::new(move |_, _, _| open()),
        cancelled: Box::new(|| CANCEL.load(Ordering::SeqCst)),
        capabilities: Box::new(Vec::new),
        format_partition: Box::new(|_, _, _, _| Err(unavailable("host format"))),
        rescan: Box::new(|_, _| Ok(())),
        pause: Box::new(|| std::thread::sleep(std::time::Duration::from_millis(200))),
    }
}

/// Le résultat dans le format de la ligne de commande : un seul lecteur, comme sur le bureau.
fn finished(result: Result<WriteOutcome, job::JobError>) -> Event {
    let failure =
        |message: String, code: Option<String>| Event::Finished(Outcome::Failed { code, message });
    match result {
        Err(error) => failure(error.message, Some(error.code.id().to_owned())),
        Ok(outcome) => {
            let line = match outcome {
                WriteOutcome::Done(done) => serde_json::to_string(&done),
                WriteOutcome::Extracted(done) => serde_json::to_string(&done),
                WriteOutcome::Plan(plan) => serde_json::to_string(&plan),
            };
            line.ok()
                .and_then(|line| parse_line(&line))
                .unwrap_or_else(|| failure("unreadable result".to_owned(), None))
        }
    }
}

pub(crate) struct Request<'a> {
    pub(crate) disk_id: &'a str,
    pub(crate) source: &'a str,
    pub(crate) sha256: Option<String>,
    pub(crate) raw: bool,
    pub(crate) mbr: bool,
}

/// Écrit et vérifie, de bout en bout. Bloque : l'application l'appelle hors du fil de l'écran, et lit la
/// progression par ailleurs.
pub(crate) fn write(
    request: &Request<'_>,
    open: impl Fn() -> Result<Box<dyn BlockDevice>, BlockError> + 'static,
) {
    let disks: Vec<Disk> = MODEL
        .lock()
        .map(|model| model.disks.clone())
        .unwrap_or_default();
    let io = android_io(open);
    let enumerate = move || Ok(disks.clone());
    let deps = Deps {
        enumerate: &enumerate,
        io: &io,
    };
    let job = WriteJob {
        target: Target::Disk(request.disk_id.to_owned()),
        source: Path::new(request.source).to_path_buf(),
        options: FilterOptions::default(),
        verify: true,
        repair_gpt: true,
        trace_io: None,
        dry_run: false,
        yes: true,
        expected_sha256: request.sha256.clone(),
        mode: if request.raw {
            WriteMode::Raw
        } else {
            WriteMode::Auto
        },
        table: if request.mbr {
            TableKind::Mbr
        } else {
            TableKind::Gpt
        },
        elevation: Elevation::Never,
        force: false,
    };
    let mut progress = |update: &Progress| {
        push(Event::Progress {
            phase: update.phase,
            done: update.bytes_done,
            total: update.bytes_total,
            rate: update.bytes_per_second,
        });
        if CANCEL.load(Ordering::SeqCst) {
            Control::Cancel
        } else {
            Control::Continue
        }
    };
    push(finished(job::write(&job, &deps, &mut progress)));
}
