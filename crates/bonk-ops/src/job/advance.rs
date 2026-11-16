use std::time::Instant;

use bonk_core::write::{Control, Phase, Progress};

/// Progress of a phase counted in bytes, shared by the copy and the read-back.
pub(in crate::job) struct Advance<'a, 'b> {
    pub(in crate::job) phase: Phase,
    pub(in crate::job) done: u64,
    pub(in crate::job) total: u64,
    pub(in crate::job) started: Instant,
    pub(in crate::job) report: &'a mut (dyn FnMut(&Progress) -> Control + 'b),
}

impl<'a, 'b> Advance<'a, 'b> {
    pub(in crate::job) fn start(
        phase: Phase,
        total: u64,
        report: &'a mut (dyn FnMut(&Progress) -> Control + 'b),
    ) -> Self {
        Self {
            phase,
            done: 0,
            total,
            started: Instant::now(),
            report,
        }
    }

    pub(in crate::job) fn add(&mut self, bytes: u64) -> bool {
        self.done += bytes;
        let elapsed = self.started.elapsed();
        let event = Progress {
            phase: self.phase,
            bytes_done: self.done,
            bytes_total: Some(self.total),
            bytes_per_second: self.done as f64 / elapsed.as_secs_f64().max(0.001),
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        };
        (self.report)(&event) != Control::Cancel
    }
}
