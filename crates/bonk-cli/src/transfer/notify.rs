use bonk_core::write::Progress;
use bonk_hal::Lang;
use bonk_hal::notify::{Notice, Notifications, Step};
use bonk_ops::job::{JobError, WriteOutcome};

use crate::errors::Code;

/// `--notify`: the same steps as the progress line, in the system's notifications.
pub(super) struct Notices(Option<Notifications>);

impl Notices {
    pub(super) fn new(on: bool) -> Self {
        Self(on.then(Notifications::start))
    }

    pub(super) fn progress(&mut self, progress: &Progress) {
        self.send(&Step::Progress {
            phase: progress.phase,
            done: progress.bytes_done,
            total: progress.bytes_total,
        });
    }

    pub(super) fn end(&mut self, result: &Result<WriteOutcome, JobError>) {
        if let Some(step) = ended(result) {
            self.send(&step);
        }
    }

    fn send(&mut self, step: &Step) {
        if let Some(notifications) = &mut self.0 {
            notifications.send(Notice::new(step, Lang::En));
        }
    }
}

fn ended(result: &Result<WriteOutcome, JobError>) -> Option<Step> {
    Some(match result {
        Ok(WriteOutcome::Plan(_)) => return None,
        Ok(WriteOutcome::Done(done)) => Step::Written {
            verified: done.verify.is_some(),
        },
        // A Windows drive is always read back file by file.
        Ok(WriteOutcome::Extracted(_)) => Step::Written { verified: true },
        Err(error) if error.code == Code::Cancelled => Step::Cancelled,
        Err(error) => Step::Failed {
            message: format!("{}: {}", error.code.id(), error.message),
        },
    })
}
