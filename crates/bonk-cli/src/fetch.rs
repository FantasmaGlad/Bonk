//! Source de `bonk write` : une adresse devient un fichier du cache, dont l'empreinte est connue avant le
//! premier octet écrit sur le support. La suite ne distingue plus un téléchargement d'un fichier local :
//! même contrôle, même écriture.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use bonk_catalog::{Advance, DownloadError, Hooks, SILENCE_LIMIT};
use bonk_core::write::{Control, Phase, Progress};
use bonk_ops::errors::Code;
use bonk_ops::job::{self, Deps, JobError, WriteJob};

/// Attente avant de réessayer un téléchargement, en pauses courtes par essai : environ 2 s de plus à chaque
/// fois, le temps que le réseau revienne.
const RETRY_PAUSES: u32 = 8;

pub(crate) struct Request<'a> {
    pub(crate) source: &'a Path,
    pub(crate) sha256_file: Option<&'a str>,
    pub(crate) cache_dir: Option<&'a Path>,
}

/// Un fichier du cache, à oublier s'il se révèle faux : le retélécharger est le seul remède.
#[derive(Debug)]
pub(crate) struct Downloaded(PathBuf);

impl Downloaded {
    pub(crate) fn forget_if_corrupt(&self, error: &JobError) {
        if error.code == Code::SourceHashMismatch
            && let Err(failure) = fs::remove_file(&self.0)
        {
            log::warn!(target: "bonk::catalog", error = failure.to_string().as_str(); "corrupt download not removed");
        }
    }
}

fn error(code: Code, message: impl std::fmt::Display) -> JobError {
    JobError {
        code,
        message: message.to_string(),
    }
}

fn download_error(failure: &DownloadError) -> JobError {
    let code = match failure {
        DownloadError::Cancelled => Code::Cancelled,
        _ => Code::DownloadFailed,
    };
    error(code, failure)
}

/// Lit l'empreinte de l'image `name` dans un fichier d'empreintes, local ou distant.
fn sha256_from_file(location: &str, name: &str) -> Result<String, JobError> {
    let text = if bonk_catalog::is_url(location) {
        bonk_catalog::fetch_text(location).map_err(|failure| download_error(&failure))?
    } else {
        fs::read_to_string(location)
            .map_err(|failure| error(Code::SourceUnreadable, format!("{location} : {failure}")))?
    };
    bonk_catalog::find_sha256(&text, name).ok_or_else(|| {
        error(
            Code::InvalidArgument,
            format!("{location} ne donne aucune empreinte SHA-256 pour {name}"),
        )
    })
}

/// Prépare `job` : l'empreinte d'un fichier d'empreintes, et pour une adresse le téléchargement. Rend le
/// fichier téléchargé, s'il y en a un. Tout ce qui refuserait l'écriture est vérifié avant de télécharger.
pub(crate) fn resolve(
    request: &Request<'_>,
    job: &mut WriteJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<Option<Downloaded>, JobError> {
    let source = request.source.to_str().filter(|s| bonk_catalog::is_url(s));
    let name = match source {
        Some(url) => bonk_catalog::file_name(url)
            .ok_or_else(|| {
                error(
                    Code::InvalidArgument,
                    format!("{url} : aucun nom de fichier lisible dans l'adresse"),
                )
            })?
            .to_owned(),
        None => request
            .source
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
    };
    if let Some(expected) = &job.expected_sha256 {
        job::normalize_sha256(expected)?;
    }
    let Some(url) = source else {
        if let Some(location) = request.sha256_file {
            job.expected_sha256 = Some(sha256_from_file(location, &name)?);
        }
        return Ok(None);
    };
    if job.expected_sha256.is_none() && request.sha256_file.is_none() {
        return Err(error(
            Code::InvalidArgument,
            "une image téléchargée exige son empreinte officielle : --sha256 ou --sha256-file",
        ));
    }
    job::precheck(job, deps)?;
    if let Some(location) = request.sha256_file {
        job.expected_sha256 = Some(sha256_from_file(location, &name)?);
    }
    let sha256 = job::normalize_sha256(job.expected_sha256.as_deref().unwrap_or_default())?;
    let cache = request
        .cache_dir
        .map(Path::to_path_buf)
        .or_else(|| bonk_hal::platform::cache_dir(&|name| std::env::var(name).ok()))
        .ok_or_else(|| {
            error(
                Code::DownloadFailed,
                "aucun dossier de cache connu : le donner avec --cache-dir",
            )
        })?;
    // Rangé sous son empreinte : une nouvelle version de même nom ne reprend jamais l'ancienne.
    let dest = cache.join("images").join(&sha256).join(&name);
    if dest.exists() {
        log::info!(target: "bonk::catalog", path = dest.display().to_string().as_str(); "image already downloaded");
    } else {
        fetch_to(url, &dest, deps, progress)?;
    }
    job.source.clone_from(&dest);
    job.expected_sha256 = Some(sha256);
    Ok(Some(Downloaded(dest)))
}

fn fetch_to(
    url: &str,
    dest: &Path,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(), JobError> {
    let started = Instant::now();
    let mut first_done = None;
    let stopped = std::cell::Cell::new(false);
    let mut on_advance = |advance: Advance| {
        let base = *first_done.get_or_insert(advance.done);
        let elapsed = started.elapsed();
        let event = Progress {
            phase: Phase::Download,
            bytes_done: advance.done,
            bytes_total: advance.total,
            bytes_per_second: (advance.done - base) as f64 / elapsed.as_secs_f64().max(0.001),
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        };
        if progress(&event) == Control::Cancel {
            stopped.set(true);
        }
    };
    let cancelled = || stopped.get() || (deps.io.cancelled)();
    let pause = |attempt: u32| {
        for _ in 0..attempt * RETRY_PAUSES {
            (deps.io.pause)();
        }
    };
    let mut hooks = Hooks {
        progress: &mut on_advance,
        cancelled: &cancelled,
        pause: &pause,
        silence_limit: SILENCE_LIMIT,
    };
    bonk_catalog::download(url, dest, &mut hooks)
        .map(drop)
        .map_err(|failure| download_error(&failure))
}
