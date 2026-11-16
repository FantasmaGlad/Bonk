use super::*;

/// Redirections follow the same rule.
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

/// Last path segment, without query or fragment. `None` if empty or encoded: a name from the network must not
/// escape the cache directory.
#[must_use]
pub fn file_name(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    let (_, after_scheme) = path.split_once("://")?;
    let (_, path) = after_scheme.split_once('/')?;
    let name = path.rsplit('/').next()?;
    let safe = !name.is_empty() && name != ".." && name != "." && !name.contains(['%', '\\', '\0']);
    safe.then_some(name)
}

/// One setting for the whole process: no download can forget it.
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
        // 4xx and 5xx are read as responses: they are handled differently.
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
    response
        .body_mut()
        .with_config()
        // ureq refuses a body that reaches its limit: exactly `limit` bytes must still pass.
        .limit(limit.saturating_add(1))
        .read_to_vec()
        .map_err(|e| DownloadError::Network(format!("{url}: {e}")))
}
