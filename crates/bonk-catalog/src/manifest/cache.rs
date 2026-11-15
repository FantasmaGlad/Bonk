use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use ring::digest::{SHA256, digest};

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fetches a file up to the given number of bytes.
pub type FetchSized<'a> = dyn Fn(&str, u64) -> Result<Vec<u8>, String> + 'a;

/// The bytes carry the signed hash of `asset`.
#[must_use]
pub fn asset_matches(asset: &Asset, bytes: &[u8]) -> bool {
    hex(digest(&SHA256, bytes).as_ref()).eq_ignore_ascii_case(&asset.sha256)
}

/// In the cache under `folder`, named by its signed hash, downloaded when missing, up to its signed size.
/// A cached copy is hashed again before use: a file changed on disk is downloaded anew.
///
/// # Errors
/// Download failure, or bytes that do not match the signed hash.
pub fn asset_path(
    base: &str,
    dir: &Path,
    folder: &str,
    asset: &Asset,
    fetch: &FetchSized<'_>,
) -> Result<PathBuf, String> {
    let name = Path::new(&asset.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let path = dir.join(folder).join(&asset.sha256).join(name);
    if fs::read(&path).is_ok_and(|bytes| asset_matches(asset, &bytes)) {
        return Ok(path);
    }
    let bytes = fetch(&format!("{base}{}", asset.path), asset.size_bytes)?;
    if !asset_matches(asset, &bytes) {
        return Err(format!("{} does not match its signed hash", asset.path));
    }
    let parent = path.parent().unwrap_or(dir);
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    replace(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// In the cache, checked by its signed hash, the cached copy included. Descriptive: `None` when missing,
/// each failure logged.
#[must_use]
pub fn icon_path(
    base: &str,
    dir: &Path,
    family: &Family,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Option<PathBuf> {
    if let Some(path) = crate::icons::cached(dir, family) {
        return Some(path);
    }
    let warn = |what: &str, error: &dyn std::fmt::Display| {
        log::warn!(target: "bonk::catalog", family = family.id.as_str(), error:% = error; "{what}");
    };
    let bytes = fetch(&format!("{base}{}", family.icon))
        .map_err(|e| warn("icon not downloaded", &e))
        .ok()?;
    crate::icons::store(dir, family, &bytes)
        .map_err(|e| warn("icon not kept", &e))
        .ok()
}
