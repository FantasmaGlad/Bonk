use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_catalog::manifest::{self, Catalog, Image, ImageSource, Loaded, TRUSTED_KEYS};
use bonk_core::vault::{VaultFamily, default_families, families_from, family_errors};
use bonk_ops::job::{JobError, WriteMode};

use crate::errors::Code;

pub(super) fn base_url(given: Option<&str>, variable: Option<String>) -> String {
    let url = given
        .map(str::to_owned)
        .or(variable)
        .unwrap_or_else(|| manifest::DEFAULT_BASE.to_owned());
    if url.ends_with('/') {
        url
    } else {
        format!("{url}/")
    }
}

pub(crate) fn refused(message: impl std::fmt::Display) -> JobError {
    JobError {
        code: Code::CatalogRefused,
        message: message.to_string(),
    }
}

pub(crate) type FetchFn = Box<dyn Fn(&str) -> Result<Vec<u8>, String>>;
pub(crate) type FetchFileFn = Box<dyn Fn(&str, u64) -> Result<Vec<u8>, String>>;

pub(crate) struct Access {
    pub(crate) keys: Vec<[u8; 32]>,
    pub(crate) fetch: FetchFn,
    pub(crate) fetch_file: FetchFileFn,
}

impl Default for Access {
    fn default() -> Self {
        Self {
            keys: TRUSTED_KEYS.to_vec(),
            fetch: Box::new(|address| {
                bonk_catalog::fetch_bytes(address).map_err(|e| e.to_string())
            }),
            fetch_file: Box::new(|address, size| {
                bonk_catalog::fetch_limited(address, size).map_err(|e| e.to_string())
            }),
        }
    }
}

pub(crate) fn load_from(
    access: &Access,
    url: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<(Loaded, PathBuf, String), JobError> {
    let base = base_url(url, std::env::var("BONK_CATALOG_URL").ok());
    let dir = crate::cache::directory(cache_dir)
        .ok_or_else(|| refused("no known cache directory: give it with --cache-dir"))?
        .join("catalog");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let loaded = manifest::load(&base, &dir, now, &access.keys, &*access.fetch).map_err(refused)?;
    if loaded.saved_copy {
        log::warn!(target: "bonk::catalog", "catalog not reachable: using the saved copy");
    }
    Ok((loaded, dir, base))
}

// No network: a command on a local file stays offline. No copy, or one refused, gives the families this
// Bonk knows.
pub(crate) fn saved_families(access: &Access, cache_dir: Option<&Path>) -> Vec<VaultFamily> {
    let Some(dir) = crate::cache::directory(cache_dir) else {
        return default_families();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    match manifest::saved(&dir.join("catalog"), now, &access.keys) {
        Ok(catalog) => families_from(&catalog.vault_families),
        Err(error) => {
            log::info!(target: "bonk::catalog", error = error.to_string().as_str(); "vault families of this Bonk");
            default_families()
        }
    }
}

/// # Errors
/// Unreadable file, not a list of families, an empty list, or a family the menu would refuse.
pub(crate) fn families_file(path: &Path) -> Result<Vec<VaultFamily>, JobError> {
    let invalid = |message: String| JobError {
        code: Code::InvalidArgument,
        message: format!("{}: {message}", path.display()),
    };
    let text = std::fs::read_to_string(path).map_err(|e| invalid(e.to_string()))?;
    let families: Vec<VaultFamily> =
        serde_json::from_str(&text).map_err(|e| invalid(e.to_string()))?;
    if families.is_empty() {
        return Err(invalid("no vault family".to_owned()));
    }
    let errors: Vec<String> = families.iter().flat_map(family_errors).collect();
    if errors.is_empty() {
        Ok(families)
    } else {
        Err(invalid(errors.join("; ")))
    }
}

pub(crate) fn load(
    url: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<(Loaded, PathBuf), JobError> {
    load_from(&Access::default(), url, cache_dir).map(|(loaded, dir, _)| (loaded, dir))
}

#[derive(Debug)]
pub(crate) struct CatalogSource {
    pub(crate) url: PathBuf,
    pub(crate) sha256: String,
    /// Set only when the address does not end with the file's name.
    pub(crate) file_name: Option<String>,
    /// Starts from a drive only written with `--mode hybrid`.
    pub(crate) hybrid: bool,
}

pub(crate) fn mode_for(requested: WriteMode, hybrid: bool) -> WriteMode {
    if hybrid && requested == WriteMode::Auto {
        WriteMode::Hybrid
    } else {
        requested
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Destination {
    Drive,
    Vault,
    Cache,
}

fn refusal(image: &Image, destination: Destination) -> Option<String> {
    let usb = image.usb_boot || image.hybrid;
    let starts = match destination {
        Destination::Drive => usb,
        Destination::Vault => usb || image.vault,
        Destination::Cache => true,
    };
    if starts {
        return None;
    }
    let why = image.note.as_deref().unwrap_or("see its publisher");
    Some(if image.vault {
        format!(
            "{} {} does not boot written to a USB drive, but from a Bonk vault: bonk vault add <disk> --image {}",
            image.name, image.version, image.id
        )
    } else {
        format!(
            "{} {} does not boot from a USB drive: {why}",
            image.name, image.version
        )
    })
}

pub(crate) fn resolve_image(
    catalog: &Catalog,
    id: &str,
    destination: Destination,
) -> Result<CatalogSource, JobError> {
    let image = catalog
        .images
        .iter()
        .find(|image| image.id == id)
        .ok_or_else(|| JobError {
            code: Code::InvalidArgument,
            message: format!("unknown image: '{id}' (images: bonk catalog list)"),
        })?;
    if let Some(message) = refusal(image, destination) {
        return Err(JobError {
            code: Code::InvalidArgument,
            message,
        });
    }
    match &image.source {
        ImageSource::Download { url, sha256, .. } => Ok(CatalogSource {
            url: PathBuf::from(url),
            sha256: sha256.clone(),
            file_name: image.file_name.clone(),
            hybrid: image.hybrid,
        }),
        ImageSource::OfficialPage { official_page } => Err(JobError {
            code: Code::InvalidArgument,
            message: format!(
                "{} {} has no direct link: download it from {official_page}, then give the file",
                image.name, image.version
            ),
        }),
    }
}

pub(crate) fn image_source(
    id: &str,
    url: Option<&str>,
    cache_dir: Option<&Path>,
    destination: Destination,
) -> Result<CatalogSource, JobError> {
    let (loaded, _) = load(url, cache_dir)?;
    resolve_image(&loaded.catalog, id, destination)
}
