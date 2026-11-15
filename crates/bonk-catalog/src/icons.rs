use std::fs;
use std::path::{Path, PathBuf};

use ring::digest::{SHA256, digest};

use crate::manifest::{Family, hex, replace};

/// Named by the signed hash: a new logo never reuses an old file.
fn file(dir: &Path, family: &Family) -> Option<PathBuf> {
    let extension = Path::new(&family.icon).extension()?.to_str()?;
    Some(
        dir.join("icons")
            .join(format!("{}.{extension}", family.icon_sha256)),
    )
}

fn matches(family: &Family, bytes: &[u8]) -> bool {
    hex(digest(&SHA256, bytes).as_ref()).eq_ignore_ascii_case(&family.icon_sha256)
}

/// The cached logo, checked again by its signed hash at each read.
#[must_use]
pub fn cached(dir: &Path, family: &Family) -> Option<PathBuf> {
    let path = file(dir, family)?;
    fs::read(&path)
        .is_ok_and(|bytes| matches(family, &bytes))
        .then_some(path)
}

/// Kept only if the bytes match the signed hash.
///
/// # Errors
/// The bytes are not the signed logo, or the cache cannot be written.
pub fn store(dir: &Path, family: &Family, bytes: &[u8]) -> Result<PathBuf, String> {
    if !matches(family, bytes) {
        return Err(format!(
            "{}: icon does not match its signed hash",
            family.id
        ));
    }
    let path = file(dir, family).ok_or("icon without extension")?;
    path.parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| replace(&path, bytes))
        .map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(icon_sha256: String) -> Family {
        serde_json::from_value(serde_json::json!({
            "id": "debian", "name": "Debian", "icon": "icons/debian.png", "icon_sha256": icon_sha256,
        }))
        .unwrap_or_else(|error| panic!("famille d'essai : {error}"))
    }

    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bonk-icons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn seul_le_logo_signe_est_garde_puis_relu() -> Result<(), String> {
        let dir = folder();
        let logo = b"png";
        let signed = family(hex(digest(&SHA256, logo).as_ref()));
        assert!(store(&dir, &signed, b"autre").is_err());
        assert_eq!(cached(&dir, &signed), None);
        let path = store(&dir, &signed, logo)?;
        assert_eq!(cached(&dir, &signed), Some(path.clone()));
        fs::write(&path, b"remplace").map_err(|e| e.to_string())?;
        assert_eq!(
            cached(&dir, &signed),
            None,
            "un logo modifié n'est plus servi"
        );
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }
}
