use std::io::Read;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use ureq::http::StatusCode;
use ureq::tls::{RootCerts, TlsConfig};
use ureq::{Agent, Proxy};

use super::{DownloadError, TEXT_LIMIT};

pub(super) fn https_required(url: &str) -> Result<bool, DownloadError> {
    if url.starts_with("https://") {
        return Ok(true);
    }
    let host = url.strip_prefix("http://").map(|rest| {
        let authority = rest.split('/').next().unwrap_or(rest);
        authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host)
    });
    match host {
        Some("127.0.0.1" | "localhost" | "[::1]") => Ok(false),
        _ => Err(DownloadError::Scheme(url.to_owned())),
    }
}

#[must_use]
pub fn is_url(text: &str) -> bool {
    text.starts_with("https://") || text.starts_with("http://")
}

static PROXY: RwLock<Option<Proxy>> = RwLock::new(None);

/// `socks5h://` only: any other scheme resolves names outside the proxy, and the anonymity Tor promises would
/// be false. An unreachable proxy is an error, never a direct connection.
///
/// # Errors
/// Malformed address, or a scheme other than `socks5h://`.
pub fn set_proxy(url: &str) -> Result<(), String> {
    if !url.starts_with("socks5h://") {
        return Err(format!("{url}: only socks5h:// is accepted"));
    }
    let proxy = Proxy::new(url).map_err(|e| format!("{url}: {e}"))?;
    *PROXY.write().unwrap_or_else(PoisonError::into_inner) = Some(proxy);
    Ok(())
}

pub(super) fn agent(https_only: bool) -> Result<Agent, DownloadError> {
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::PlatformVerifier)
        .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .build();
    Ok(Agent::config_builder()
        .https_only(https_only)
        .proxy(PROXY.read().unwrap_or_else(PoisonError::into_inner).clone())
        .tls_config(tls)
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_response(Some(Duration::from_secs(60)))
        .http_status_as_error(false)
        .user_agent(concat!("bonk/", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent())
}

/// # Errors
/// Refused address, status other than 200, network, or text too long.
pub fn fetch_text(url: &str) -> Result<String, DownloadError> {
    String::from_utf8(fetch_bytes(url)?).map_err(|e| DownloadError::Network(format!("{url}: {e}")))
}

/// Manifest, signature, logo.
///
/// # Errors
/// Refused address, status other than 200, network, or file too long.
pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, DownloadError> {
    fetch_limited(url, TEXT_LIMIT)
}

/// Like [`fetch_bytes`], for a file whose size is known in advance, such as one the signed catalog names.
///
/// # Errors
/// Refused address, status other than 200, network, or more than `limit` bytes.
pub fn fetch_limited(url: &str, limit: u64) -> Result<Vec<u8>, DownloadError> {
    let agent = agent(https_required(url)?)?;
    let mut response = agent
        .get(url)
        .call()
        .map_err(|e| DownloadError::Network(format!("{url}: {e}")))?;
    let status = response.status();
    if status != StatusCode::OK {
        return Err(DownloadError::Refused {
            url: url.to_owned(),
            status: status.as_u16(),
        });
    }
    let network = |e: &dyn std::fmt::Display| DownloadError::Network(format!("{url}: {e}"));
    let mut bytes = Vec::new();
    response
        .body_mut()
        .with_config()
        // ureq counts the bytes received, before decompression, and refuses a body that reaches its limit.
        .limit(compressed_bound(limit).saturating_add(1))
        .reader()
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| network(&e))?;
    if bytes.len() as u64 > limit {
        return Err(network(&format!(
            "more than {limit} bytes once decompressed"
        )));
    }
    Ok(bytes)
}

/// Largest gzip stream for `len` bytes: 18 bytes of framing and 5 per stored deflate block of 65535 bytes.
fn compressed_bound(len: u64) -> u64 {
    len.saturating_add(18)
        .saturating_add(len.div_ceil(65_535).max(1).saturating_mul(5))
}
