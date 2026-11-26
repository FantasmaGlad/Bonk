use std::collections::{HashMap, VecDeque};
use std::io::{self, Read};
use std::sync::{Mutex, PoisonError};

use ureq::http::{StatusCode, header};
use ureq::{Agent, ResponseExt};

use super::partial::parse_content_range;
use super::stream::{header_text, range_validator};
use super::{DownloadError, agent, https_required};

const BLOCK: u64 = 256 * 1024;
/// Bounds memory: the directories and boot images of an ISO fit well within it.
const KEPT_BLOCKS: usize = 64;
const ATTEMPTS: u32 = 3;

/// A file on a server read by range requests, block by block, without downloading it.
pub struct RemoteFile {
    agent: Agent,
    url: String,
    len: u64,
    validator: Option<String>,
    blocks: Mutex<Blocks>,
}

impl std::fmt::Debug for RemoteFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteFile")
            .field("url", &self.url)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

#[derive(Default)]
struct Blocks {
    kept: HashMap<u64, Vec<u8>>,
    order: VecDeque<u64>,
}

impl Blocks {
    fn keep(&mut self, index: u64, bytes: Vec<u8>) {
        if self.order.len() == KEPT_BLOCKS
            && let Some(oldest) = self.order.pop_front()
        {
            self.kept.remove(&oldest);
        }
        self.order.push_back(index);
        self.kept.insert(index, bytes);
    }
}

struct Part {
    /// Where the redirects ended.
    location: String,
    total: u64,
    validator: Option<String>,
    bytes: Vec<u8>,
}

impl RemoteFile {
    /// # Errors
    /// Refused address or status, network, or a server that does not answer range requests.
    pub fn open(url: &str) -> Result<Self, DownloadError> {
        let agent = agent(https_required(url)?)?;
        let first = fetch_part(&agent, url, 0, BLOCK, None)?;
        let mut blocks = Blocks::default();
        blocks.keep(0, first.bytes);
        Ok(Self {
            agent,
            // A redirector may send each request to another mirror, whose validator differs: every block
            // is asked of the mirror that answered first.
            url: first.location,
            len: first.total,
            validator: first.validator,
            blocks: Mutex::new(blocks),
        })
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.len
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// # Errors
    /// Beyond the end of the file, network, or a file changed on the server since it was opened.
    pub fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        let end = offset
            .checked_add(buf.len() as u64)
            .filter(|end| *end <= self.len)
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        let mut at = offset;
        while at < end {
            let index = at / BLOCK;
            let from = at % BLOCK;
            let take = (BLOCK - from).min(end - at);
            let done = usize::try_from(at - offset).map_err(io::Error::other)?;
            let out = buf
                .get_mut(done..done + take as usize)
                .ok_or(io::ErrorKind::UnexpectedEof)?;
            self.copy_from_block(index, from as usize, out)?;
            at += take;
        }
        Ok(())
    }

    fn copy_from_block(&self, index: u64, from: usize, out: &mut [u8]) -> io::Result<()> {
        let mut blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
        if !blocks.kept.contains_key(&index) {
            let start = index * BLOCK;
            let part = fetch_part(
                &self.agent,
                &self.url,
                start,
                BLOCK,
                self.validator.as_deref(),
            )
            .map_err(io::Error::other)?;
            if part.total != self.len {
                return Err(io::Error::other(format!(
                    "{}: the file changed on the server",
                    self.url
                )));
            }
            blocks.keep(index, part.bytes);
        }
        let bytes = blocks
            .kept
            .get(&index)
            .and_then(|block| block.get(from..from + out.len()))
            .ok_or(io::ErrorKind::UnexpectedEof)?;
        out.copy_from_slice(bytes);
        Ok(())
    }
}

fn fetch_part(
    agent: &Agent,
    url: &str,
    start: u64,
    len: u64,
    validator: Option<&str>,
) -> Result<Part, DownloadError> {
    let mut attempt = 1;
    loop {
        match request_part(agent, url, start, len, validator) {
            Err(DownloadError::Network(message)) if attempt < ATTEMPTS => {
                log::warn!(target: "bonk::download", error = message.as_str(), attempt = attempt; "range request failed, retried");
                attempt += 1;
            }
            result => return result,
        }
    }
}

fn request_part(
    agent: &Agent,
    url: &str,
    start: u64,
    len: u64,
    known: Option<&str>,
) -> Result<Part, DownloadError> {
    let mut request = agent
        .get(url)
        .header(header::ACCEPT_ENCODING, "identity")
        .header(header::RANGE, format!("bytes={start}-{}", start + len - 1));
    if let Some(known) = known {
        request = request.header(header::IF_RANGE, known);
    }
    let mut response = request
        .call()
        .map_err(|e| DownloadError::Network(format!("{url}: {e}")))?;
    partial_status(response.status(), url, known.is_some())?;
    let location = response.get_uri().to_string();
    let headers = response.headers();
    let validator = range_validator(headers);
    let place = header_text(headers, header::CONTENT_RANGE).and_then(|v| parse_content_range(&v));
    let Some((first, Some(total))) = place else {
        return Err(inconsistent(
            url,
            "partial answer without its place in the file",
        ));
    };
    if first != start || start >= total {
        return Err(inconsistent(url, "partial answer at another place"));
    }
    let bytes = read_body(response.body_mut(), url, len.min(total - start))?;
    Ok(Part {
        location,
        total,
        validator,
        bytes,
    })
}

fn read_body(body: &mut ureq::Body, url: &str, expected: u64) -> Result<Vec<u8>, DownloadError> {
    let mut bytes = Vec::new();
    body.with_config()
        .limit(expected.saturating_add(1))
        .reader()
        .take(expected.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| DownloadError::Network(format!("{url}: {e}")))?;
    if bytes.len() as u64 != expected {
        return Err(inconsistent(url, "partial answer of another length"));
    }
    Ok(bytes)
}

fn partial_status(status: StatusCode, url: &str, validated: bool) -> Result<(), DownloadError> {
    // The whole file in answer: never read, an ISO is gigabytes.
    if status == StatusCode::OK {
        return Err(inconsistent(
            url,
            if validated {
                "the file changed on the server"
            } else {
                "the server does not answer partial requests"
            },
        ));
    }
    if status != StatusCode::PARTIAL_CONTENT {
        return Err(DownloadError::Refused {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }
    Ok(())
}

fn inconsistent(url: &str, what: &str) -> DownloadError {
    DownloadError::Inconsistent(format!("{url}: {what}"))
}
