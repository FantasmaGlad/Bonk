use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use ureq::Agent;

mod http;
mod partial;
mod remote;
mod stream;

use http::*;
pub use http::{fetch_bytes, fetch_limited, fetch_text, is_url, set_proxy};
use partial::*;
pub use remote::RemoteFile;
use stream::*;

const ATTEMPTS_WITHOUT_PROGRESS: u32 = 5;
const CHUNK: usize = 256 * 1024;
/// Chunks queued between the two threads: bounds memory when the disk is slower than the network.
const QUEUED_CHUNKS: usize = 8;
const POLL: Duration = Duration::from_millis(100);
pub const SILENCE_LIMIT: Duration = Duration::from_secs(60);
/// A checksum file fits well within this; the cap guards against a wrong URL.
const TEXT_LIMIT: u64 = 1 << 20;

#[derive(Debug)]
pub enum DownloadError {
    Scheme(String),
    Refused { url: String, status: u16 },
    Network(String),
    Inconsistent(String),
    Io(io::Error),
    Cancelled,
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scheme(url) => write!(
                f,
                "{url}: only https:// is accepted (http:// only to this machine)"
            ),
            Self::Refused { url, status } => write!(f, "{url}: the server answers {status}"),
            Self::Network(message) | Self::Inconsistent(message) => f.write_str(message),
            Self::Io(err) => write!(f, "download file: {err}"),
            Self::Cancelled => f.write_str("download cancelled, can be resumed"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl DownloadError {
    /// A rolling distribution deletes yesterday's snapshot as it publishes today's: the signed catalog only
    /// names the new file at its next publication.
    #[must_use]
    pub fn of_catalog_image(&self) -> String {
        match self {
            Self::Refused {
                status: 404 | 410, ..
            } => format!(
                "{self}: the publisher replaced or removed this file; the catalog takes its new address at its next publication, try again later"
            ),
            _ => self.to_string(),
        }
    }
}

impl From<io::Error> for DownloadError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Advance {
    pub done: u64,
    pub total: Option<u64>,
}

pub struct Hooks<'a> {
    pub progress: &'a mut dyn FnMut(Advance),
    pub cancelled: &'a dyn Fn() -> bool,
    pub pause: &'a dyn Fn(u32),
    pub silence_limit: Duration,
}

enum Attempt {
    Complete,
    Retry(String),
}

/// Resumes `<dest>.part` if present. Retries alone while the network gives new bytes; after a final cut the
/// partial file stays for next time.
///
/// # Errors
/// Refused address or response, network after every attempt, local write, or cancel.
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
    match accept(head, url, partial)? {
        Accepted::Body(total) => receive(&events, url, total, partial, hooks),
        Accepted::Done(attempt) => Ok(attempt),
    }
}

enum Accepted {
    Body(Option<u64>),
    Done(Attempt),
}

fn accept(head: Head, url: &str, partial: &mut Partial) -> Result<Accepted, DownloadError> {
    let from = partial.done;
    match head.status {
        200 => {
            // Resume refused or file changed: start over.
            partial.restart()?;
            partial.remember(head.validator)?;
            Ok(Accepted::Body(head.content_length))
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
            Ok(Accepted::Body(total))
        }
        416 => {
            let total = head.content_range.as_deref().and_then(unsatisfied_total);
            if total == Some(from) {
                return Ok(Accepted::Done(Attempt::Complete));
            }
            partial.restart()?;
            Ok(Accepted::Done(Attempt::Retry(format!(
                "{url}: partial file longer than the original"
            ))))
        }
        status if status >= 500 || status == 408 || status == 429 => Ok(Accepted::Done(
            Attempt::Retry(format!("{url}: the server answers {status}")),
        )),
        status => Err(DownloadError::Refused {
            url: url.to_owned(),
            status,
        }),
    }
}

fn receive(
    events: &Receiver<Event>,
    url: &str,
    total: Option<u64>,
    partial: &mut Partial,
    hooks: &mut Hooks<'_>,
) -> Result<Attempt, DownloadError> {
    loop {
        match next_event(events, hooks)? {
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
