use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_catalog::manifest::{self, Catalog, Image, ImageSource, Loaded, TRUSTED_KEYS};
use bonk_ops::job::JobError;

use crate::errors::Code;

/// The given address, else the variable, else Bonk's; always ending with `/`.
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
/// A file named by the catalog, read up to its signed size.
pub(crate) type FetchFileFn = Box<dyn Fn(&str, u64) -> Result<Vec<u8>, String>>;

/// Keys the signature must match and how files are fetched: injectable for tests.
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

/// Network, signature, then the saved copy when the network fails. Also returns the cache directory and the
/// address the catalog's files are relative to.
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

pub(crate) fn load(
    url: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<(Loaded, PathBuf), JobError> {
    load_from(&Access::default(), url, cache_dir).map(|(loaded, dir, _)| (loaded, dir))
}

/// What a catalog image gives a download.
#[derive(Debug)]
pub(crate) struct CatalogSource {
    pub(crate) url: PathBuf,
    pub(crate) sha256: String,
    /// Set only when the address does not end with the file's name.
    pub(crate) file_name: Option<String>,
}

/// What the image is fetched for: each use accepts what can start from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Destination {
    Drive,
    Vault,
    /// Downloaded only: kept in the cache.
    Cache,
}

fn refusal(image: &Image, destination: Destination) -> Option<String> {
    let usb = image.usb_boot;
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
