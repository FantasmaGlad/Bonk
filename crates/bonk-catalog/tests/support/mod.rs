// Each test crate uses only part of these helpers.
#![allow(dead_code)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const ETAG: &str = "\"v1\"";

#[derive(Clone, PartialEq)]
pub enum Reply {
    Serve,
    CutAfter(usize),
    IgnoreRange,
    WrongStart,
    Mute,
    Status(u16),
    Gzip,
    Raw(Vec<u8>),
    /// `/image.iso` sends to `/miroir.iso`, as a mirror redirector does at each request.
    Redirect,
}

pub struct Server {
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Server {
    pub fn start(body: Vec<u8>, script: impl Fn(usize) -> Reply + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let count = AtomicUsize::new(0);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let n = count.fetch_add(1, Ordering::SeqCst);
                let reply = script(n);
                let text = read_request(&stream);
                seen.lock().unwrap().push(text.clone());
                let body = body.clone();
                std::thread::spawn(move || answer(stream, &text, &body, reply));
            }
        });
        Self { port, requests }
    }

    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}/image.iso", self.port)
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

fn read_request(stream: &TcpStream) -> String {
    let mut reader = BufReader::new(stream);
    let mut text = String::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            return text;
        }
        text.push_str(&line.to_ascii_lowercase());
    }
}

fn requested_range(request: &str) -> Option<(usize, Option<usize>)> {
    let line = request.lines().find(|l| l.starts_with("range: bytes="))?;
    let (start, end) = line.trim_start_matches("range: bytes=").split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()))
}

pub fn requested_start(request: &str) -> Option<usize> {
    requested_range(request).map(|(start, _)| start)
}

fn answer_gzip(mut stream: TcpStream, gzip: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-encoding: gzip\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        gzip.len()
    );
    let _ = stream.write_all(gzip);
}

fn answer(mut stream: TcpStream, request: &str, body: &[u8], reply: Reply) {
    let (start, end) = requested_range(request).unzip();
    let end = end.flatten();
    let fresh = request.contains(&format!("if-range: {ETAG}"));
    let honour = reply != Reply::IgnoreRange && (fresh || !request.contains("if-range:"));
    match reply {
        Reply::Mute => {
            std::thread::sleep(Duration::from_secs(5));
            return;
        }
        Reply::Status(status) => {
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
            return;
        }
        Reply::Gzip if request.contains("accept-encoding: gzip") => {
            return answer_gzip(stream, &gzip_stored(body));
        }
        Reply::Raw(gzip) => return answer_gzip(stream, &gzip),
        Reply::Redirect if request.starts_with("get /image.iso ") => {
            let _ = write!(
                stream,
                "HTTP/1.1 302 F\r\nlocation: /miroir.iso\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            );
            return;
        }
        Reply::WrongStart => {
            let _ = write!(
                stream,
                "HTTP/1.1 206 P\r\ncontent-range: bytes 1-1/{}\r\ncontent-length: 1\r\nconnection: close\r\n\r\nx",
                body.len()
            );
            return;
        }
        _ => {}
    }
    let served = match start {
        Some(start) if honour && start >= body.len() => {
            let _ = write!(
                stream,
                "HTTP/1.1 416 R\r\ncontent-range: bytes */{}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                body.len()
            );
            return;
        }
        Some(start) if honour => {
            let last = end.map_or(body.len() - 1, |end| end.min(body.len() - 1));
            let _ = write!(
                stream,
                "HTTP/1.1 206 P\r\netag: {ETAG}\r\ncontent-range: bytes {start}-{last}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len(),
                last + 1 - start
            );
            &body[start..=last]
        }
        _ => {
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\netag: {ETAG}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            body
        }
    };
    let served = match reply {
        Reply::CutAfter(n) => &served[..n.min(served.len())],
        _ => served,
    };
    let _ = stream.write_all(served);
}

pub struct Temp(pub PathBuf);

impl Temp {
    pub fn new(tag: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "bonk-telechargement-{}-{tag}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
    pub fn dest(&self) -> PathBuf {
        self.0.join("image.iso")
    }
    pub fn part(&self) -> PathBuf {
        self.0.join("image.iso.part")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn gzip_stored(body: &[u8]) -> Vec<u8> {
    let crc = body.iter().fold(!0u32, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |c, _| {
            if c & 1 == 1 {
                (c >> 1) ^ 0xEDB8_8320
            } else {
                c >> 1
            }
        })
    });
    let len = u16::try_from(body.len()).unwrap();
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff, 1];
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&(!len).to_le_bytes());
    out.extend_from_slice(body);
    out.extend_from_slice(&(!crc).to_le_bytes());
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    out
}

pub fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}
