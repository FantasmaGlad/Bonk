use super::*;

pub(super) struct Head {
    pub(super) status: u16,
    pub(super) content_length: Option<u64>,
    pub(super) content_range: Option<String>,
    pub(super) validator: Option<String>,
}

pub(super) enum Event {
    Head(Head),
    Chunk(Vec<u8>),
    End,
    Failed(String),
}

pub(super) fn header_text(headers: &ureq::http::HeaderMap, name: header::HeaderName) -> Option<String> {
    headers.get(name)?.to_str().ok().map(str::to_owned)
}

/// A response read on its own thread: head, chunks, then `End` or `Failed`. Stops as soon as the caller lets
/// go.
pub(super) fn connect(agent: Agent, url: String, from: u64, validator: Option<String>) -> Receiver<Event> {
    let (sender, receiver) = mpsc::sync_channel(QUEUED_CHUNKS);
    std::thread::spawn(move || {
        if let Err(message) = read_response(&agent, &url, from, validator.as_deref(), &sender) {
            let _ = sender.send(Event::Failed(message));
        }
    });
    receiver
}

pub(super) fn read_response(
    agent: &Agent,
    url: &str,
    from: u64,
    validator: Option<&str>,
    sender: &SyncSender<Event>,
) -> Result<(), String> {
    let mut request = agent.get(url);
    if from > 0 {
        request = request.header(header::RANGE, format!("bytes={from}-"));
        if let Some(validator) = validator {
            request = request.header(header::IF_RANGE, validator);
        }
    }
    let response = request.call().map_err(|e| format!("{url}: {e}"))?;
    let headers = response.headers();
    // A weak ETag does not guarantee the same bytes: `If-Range` does not accept it.
    let validator = header_text(headers, header::ETAG)
        .filter(|tag| !tag.starts_with("W/"))
        .or_else(|| header_text(headers, header::LAST_MODIFIED));
    let head = Head {
        status: response.status().as_u16(),
        content_length: header_text(headers, header::CONTENT_LENGTH).and_then(|n| n.parse().ok()),
        content_range: header_text(headers, header::CONTENT_RANGE),
        validator,
    };
    if sender.send(Event::Head(head)).is_err() {
        return Ok(());
    }
    let mut body = response.into_body();
    let mut reader = body.with_config().limit(u64::MAX).reader();
    loop {
        let mut chunk = vec![0; CHUNK];
        let n = reader.read(&mut chunk).map_err(|e| format!("{url}: {e}"))?;
        if n == 0 {
            let _ = sender.send(Event::End);
            return Ok(());
        }
        chunk.truncate(n);
        if sender.send(Event::Chunk(chunk)).is_err() {
            return Ok(());
        }
    }
}
