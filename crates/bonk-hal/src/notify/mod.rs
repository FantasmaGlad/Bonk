use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bonk_core::write::Phase;

use crate::Lang;

mod text;
mod throttle;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as current;
#[cfg(windows)]
use windows as current;

pub use text::step_label;
pub use throttle::Throttle;

/// Where an operation stands, as the notification says it.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Progress {
        phase: Phase,
        done: u64,
        total: Option<u64>,
    },
    Written {
        verified: bool,
    },
    Downloaded {
        path: String,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

/// One operation has one notification, replaced in place at each notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub title: String,
    pub body: String,
    pub percent: Option<u8>,
    /// The operation ended: shown whatever the pace, and kept on screen.
    pub last: bool,
}

impl Notice {
    #[must_use]
    pub fn new(step: &Step, lang: Lang) -> Self {
        text::notice(step, lang)
    }
}

/// Shows notices outside the window; each system has its own.
pub trait Notifier {
    fn show(&mut self, notice: &Notice);

    /// Nothing more for now: a system that shows an icon while it works removes it.
    fn idle(&mut self) {}
}

/// After the last notice, how long the system keeps what it showed.
const IDLE_AFTER: Duration = Duration::from_secs(10);

/// Notices go to a thread of their own: the system call may block, the caller never does.
#[derive(Debug)]
pub struct Notifications {
    sender: Option<Sender<Notice>>,
    thread: Option<JoinHandle<()>>,
    throttle: Throttle,
    previous: Option<Notice>,
}

impl Notifications {
    #[must_use]
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("bonk-notify".to_owned())
            .spawn(move || serve(receiver))
            .inspect_err(|error| {
                log::warn!(target: "bonk::hal", error:% = error; "no notifications: thread refused");
            })
            .ok();
        Self {
            sender: Some(sender),
            thread,
            throttle: Throttle::default(),
            previous: None,
        }
    }

    /// Dropped when it repeats the previous one, or comes less than a second after it within the same step.
    pub fn send(&mut self, notice: Notice) {
        if self.previous.as_ref() == Some(&notice) || !self.throttle.admit(&notice, Instant::now())
        {
            return;
        }
        if let Some(sender) = &self.sender {
            let _ = sender.send(notice.clone());
        }
        self.previous = Some(notice);
    }
}

/// The last notice is shown before a command line process exits.
impl Drop for Notifications {
    fn drop(&mut self) {
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(receiver: Receiver<Notice>) {
    let mut notifier = system();
    loop {
        match receiver.recv_timeout(IDLE_AFTER) {
            Ok(notice) => notifier.show(&notice),
            Err(mpsc::RecvTimeoutError::Timeout) => notifier.idle(),
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    notifier.idle();
}

#[cfg(any(target_os = "linux", windows))]
fn system() -> Box<dyn Notifier> {
    Box::new(current::SystemNotifier::default())
}

/// macOS needs a signed application; Android shows its own from the app.
#[cfg(not(any(target_os = "linux", windows)))]
fn system() -> Box<dyn Notifier> {
    struct Silent;
    impl Notifier for Silent {
        fn show(&mut self, _: &Notice) {}
    }
    Box::new(Silent)
}
