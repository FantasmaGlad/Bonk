use std::fs;
use std::path::Path;

use super::verify::*;
use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub catalog: Catalog,
    /// Saved copy, the network having failed. Still subject to the signature and the expiry.
    pub saved_copy: bool,
}

const MANIFEST: &str = "catalog.json";
const SIGNATURE: &str = "catalog.json.sig";
const SEEN: &str = "seen";

pub(super) fn seen_sequence(dir: &Path) -> u64 {
    fs::read_to_string(dir.join(SEEN))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Never leaves a half-written file: a cut keeps the old one.
pub(crate) fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let partial = path.with_extension("part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

/// Under `base` or, when only the network failed, the last verified copy. A file received but refused stays
/// an error: silently falling back would hide tampering.
///
/// # Errors
/// File refused by [`verify`], or neither network nor copy.
pub fn load(
    base: &str,
    dir: &Path,
    now_unix: u64,
    keys: &[[u8; 32]],
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<Loaded, CatalogError> {
    let trust = Trust {
        keys,
        now_unix,
        seen_sequence: seen_sequence(dir),
    };
    let received = fetch(&format!("{base}{MANIFEST}"))
        .and_then(|manifest| Ok((manifest, fetch(&format!("{base}{SIGNATURE}"))?)));
    match received {
        Ok((manifest, signature)) => {
            let catalog = verify(&manifest, &signature, &trust)?;
            // The cache is only a fallback: failing to write it does not stop reading the catalog.
            let saved = fs::create_dir_all(dir)
                .and_then(|()| replace(&dir.join(MANIFEST), &manifest))
                .and_then(|()| replace(&dir.join(SIGNATURE), &signature))
                .and_then(|()| replace(&dir.join(SEEN), catalog.sequence.to_string().as_bytes()));
            if let Err(error) = saved {
                log::warn!(target: "bonk::catalog", error = error.to_string().as_str(); "catalog copy not saved");
            }
            Ok(Loaded {
                catalog,
                saved_copy: false,
            })
        }
        Err(detail) => {
            let copy = fs::read(dir.join(MANIFEST))
                .and_then(|manifest| Ok((manifest, fs::read(dir.join(SIGNATURE))?)));
            let (manifest, signature) = copy.map_err(|_| CatalogError::Unavailable(detail))?;
            Ok(Loaded {
                catalog: verify(&manifest, &signature, &trust)?,
                saved_copy: true,
            })
        }
    }
}
