use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bonk_core::write::{Control, Progress, hash_stream};
use bonk_ops::errors::Code;
use bonk_ops::job::{self, Deps, JobError};
use clap::Args;
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::report;
use crate::transfer::{self, ProgressMode};

const HASH_CHUNK: usize = 4 << 20;

#[derive(Debug, Args)]
pub(crate) struct DownloadArgs {
    /// Image identifier (see `bonk catalog list`)
    #[arg(value_name = "ID")]
    id: String,
    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL")]
    catalog_url: Option<String>,
    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// How progress is shown
    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    progress: ProgressMode,
    /// Result as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Downloaded {
    id: String,
    pub(crate) path: String,
    sha256: String,
    bytes: u64,
    /// Kept by an earlier download: checked again, nothing fetched.
    already_kept: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct DownloadDone {
    schema_version: u32,
    event: &'static str,
    download: Downloaded,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(DownloadDone)).unwrap_or_default()
}

pub(crate) fn run(
    args: &DownloadArgs,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let mode = transfer::progress_mode(args.progress, quiet);
    let result = {
        let mut sink = transfer::sink(mode, &mut *out, &mut *err);
        download(args, deps, &mut sink)
    };
    transfer::end_progress_line(mode, err);
    finish(result, mode, args.json, out, err)
}

fn download(
    args: &DownloadArgs,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<Downloaded, JobError> {
    let request = Request {
        id: &args.id,
        catalog_url: args.catalog_url.as_deref(),
        cache_dir: args.cache_dir.as_deref(),
        destination: super::Destination::Cache,
    };
    fetch_checked(&request, deps, progress)
}

pub(crate) struct Request<'a> {
    pub(crate) id: &'a str,
    pub(crate) catalog_url: Option<&'a str>,
    pub(crate) cache_dir: Option<&'a Path>,
    pub(crate) destination: super::Destination,
}

pub(crate) fn fetch_checked(
    request: &Request<'_>,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<Downloaded, JobError> {
    let cache_dir = request.cache_dir;
    let source = super::image_source(
        request.id,
        request.catalog_url,
        cache_dir,
        request.destination,
    )?;
    let url = source.url.to_string_lossy();
    let sha256 = source.sha256;
    let named = crate::fetch::Named {
        url: &url,
        file_name: source.file_name.as_deref(),
    };
    let (dest, already_kept) =
        crate::fetch::kept_or_fetched(named, cache_dir, &sha256, deps, progress)?;
    let bytes = check(&dest, &sha256, progress)?;
    Ok(Downloaded {
        id: request.id.to_owned(),
        path: dest.display().to_string(),
        sha256,
        bytes,
        already_kept,
    })
}

/// A file that does not match its hash is removed: only a new download can repair it.
fn check(
    path: &Path,
    expected: &str,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<u64, JobError> {
    let unreadable = |failure: io::Error| JobError {
        code: Code::SourceUnreadable,
        message: format!("{}: {failure}", path.display()),
    };
    let mut file = File::open(path).map_err(unreadable)?;
    let size = file.metadata().map_err(unreadable)?.len();
    let actual =
        hash_stream(&mut file, Some(size), HASH_CHUNK, progress).map_err(|failure| JobError {
            code: Code::SourceUnreadable,
            message: failure.to_string(),
        })?;
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(size);
    }
    if let Err(failure) = fs::remove_file(path) {
        log::warn!(target: "bonk::catalog", error = failure.to_string().as_str(); "corrupt download not removed");
    }
    Err(JobError {
        code: Code::SourceHashMismatch,
        message: format!(
            "{}: SHA-256 {actual}, the catalog gives {expected}; the file was removed",
            path.display()
        ),
    })
}

fn finish(
    result: Result<Downloaded, JobError>,
    mode: ProgressMode,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let download = match result {
        Ok(download) => download,
        Err(error) => {
            if mode == ProgressMode::Json {
                transfer::error_event(&error, out);
            }
            return report(err, error.code, error.message);
        }
    };
    let written = if mode == ProgressMode::Json || json {
        let done = DownloadDone {
            schema_version: job::SCHEMA_VERSION,
            event: "result",
            download,
        };
        transfer::print_json(out, &done, mode == ProgressMode::Json)
    } else {
        writeln!(out, "{}", download.path)
    };
    match written {
        Ok(()) => 0,
        Err(error) => report(err, Code::OutputFailed, error),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]

    use super::*;

    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn kept(name: &str, content: &[u8]) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("bonk-download-{}-{name}", std::process::id()));
        fs::write(&path, content).expect("image écrite");
        path
    }

    #[test]
    fn une_image_conforme_est_gardee_avec_sa_taille() {
        let path = kept("ok", b"abc");
        let size = check(&path, ABC_SHA256, &mut |_| Control::Continue).expect("empreinte juste");
        assert_eq!(size, 3);
        assert!(path.exists());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn une_image_alteree_est_refusee_et_retiree() {
        let path = kept("alteree", b"abd");
        let error = check(&path, ABC_SHA256, &mut |_| Control::Continue).expect_err("refusée");
        assert_eq!(error.code, Code::SourceHashMismatch);
        assert!(!path.exists());
    }
}
