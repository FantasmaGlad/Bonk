use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use bonk_hal::format_size;
use clap::{Args, Subcommand};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::{Code, report};
use bonk_ops::job;

const IMAGES: &str = "images";

/// The given directory, else the user's cache directory.
pub(crate) fn directory(given: Option<&Path>) -> Option<PathBuf> {
    given
        .map(Path::to_path_buf)
        .or_else(|| bonk_hal::platform::cache_dir(&|name| std::env::var(name).ok()))
}

/// Stored under its hash: a new version with the same name never resumes the old one.
pub(crate) fn image_path(cache: &Path, sha256: &str, name: &str) -> PathBuf {
    cache.join(IMAGES).join(sha256).join(name)
}

#[derive(Debug, Subcommand)]
pub(crate) enum CacheAction {
    /// List the kept images, incomplete downloads included
    List(ListArgs),
    /// Remove the kept images, or only the one with the given hash; an address is downloaded again on the
    /// next write
    Clean(CleanArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ListArgs {
    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Result as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct CleanArgs {
    /// SHA-256 of the one image to remove; without it, all are removed
    #[arg(value_name = "SHA256")]
    sha256: Option<String>,
    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Say what would be removed without removing anything
    #[arg(long)]
    dry_run: bool,
    /// Result as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Entry {
    name: String,
    sha256: String,
    bytes: u64,
    /// Incomplete download, resumed on the next write.
    partial: bool,
    /// Last change, in seconds since 1970; null when the system does not say.
    modified: Option<u64>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct CacheReport {
    schema_version: u32,
    directory: String,
    entries: Vec<Entry>,
    total_bytes: u64,
    /// Bytes removed; null for a list.
    removed_bytes: Option<u64>,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(CacheReport)).unwrap_or_default()
}

/// Real hash directories, links not followed: a link placed there never sends a removal elsewhere.
fn hash_directories(cache: &Path) -> io::Result<Vec<(String, PathBuf)>> {
    let root = cache.join(IMAGES);
    let reader = match fs::read_dir(&root) {
        Ok(reader) => reader,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut found = Vec::new();
    for entry in reader {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if bonk_catalog::is_sha256(&name) && entry.file_type()?.is_dir() {
            found.push((name, entry.path()));
        }
    }
    found.sort();
    Ok(found)
}

fn read_entries(cache: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for (sha256, directory) in hash_directories(cache)? {
        for file in fs::read_dir(directory)? {
            let file = file?;
            let name = file.file_name().to_string_lossy().into_owned();
            if !file.file_type()?.is_file() || name.ends_with(".validateur") {
                continue;
            }
            let metadata = file.metadata()?;
            entries.push(Entry {
                partial: name.ends_with(".part"),
                bytes: metadata.len(),
                modified: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|age| age.as_secs()),
                name,
                sha256: sha256.clone(),
            });
        }
    }
    Ok(entries)
}

pub(crate) fn total_bytes(cache: &Path) -> io::Result<u64> {
    Ok(read_entries(cache)?.iter().map(|entry| entry.bytes).sum())
}

/// `only`: the one hash directory to remove; the others stay.
fn remove_files(cache: &Path, only: Option<&str>) -> io::Result<()> {
    for (sha256, directory) in hash_directories(cache)? {
        if only.is_none_or(|kept| kept == sha256) {
            fs::remove_dir_all(directory)?;
        }
    }
    Ok(())
}

fn print(report: &CacheReport, json: bool, out: &mut dyn Write) -> io::Result<()> {
    if json {
        serde_json::to_writer_pretty(&mut *out, report).map_err(io::Error::from)?;
        return writeln!(out);
    }
    for entry in &report.entries {
        let state = if entry.partial { " (incomplete)" } else { "" };
        writeln!(
            out,
            "{}  {}{state}",
            format_size(entry.bytes, bonk_hal::Lang::En),
            entry.name
        )?;
    }
    match report.removed_bytes {
        Some(bytes) => writeln!(out, "Removed: {}", format_size(bytes, bonk_hal::Lang::En)),
        None => writeln!(
            out,
            "Cache: {} in {}",
            format_size(report.total_bytes, bonk_hal::Lang::En),
            report.directory
        ),
    }
}

pub(crate) fn run(action: &CacheAction, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let (given, json, clean) = match action {
        CacheAction::List(args) => (args.cache_dir.as_deref(), args.json, None),
        CacheAction::Clean(args) => (args.cache_dir.as_deref(), args.json, Some(args.dry_run)),
    };
    let only = match action {
        CacheAction::Clean(CleanArgs {
            sha256: Some(text), ..
        }) => match job::normalize_sha256(text) {
            Ok(sha256) => Some(sha256),
            Err(error) => return report(err, error.code, error.message),
        },
        _ => None,
    };
    let Some(cache) = directory(given) else {
        return report(
            err,
            Code::InvalidArgument,
            "no known cache directory: give it with --cache-dir",
        );
    };
    let done = read_entries(&cache).and_then(|mut entries| {
        if let Some(sha256) = &only {
            entries.retain(|entry| &entry.sha256 == sha256);
        }
        let total_bytes = entries.iter().map(|entry| entry.bytes).sum();
        let removed_bytes = match clean {
            Some(false) => {
                remove_files(&cache, only.as_deref())?;
                Some(total_bytes)
            }
            Some(true) | None => None,
        };
        Ok(CacheReport {
            schema_version: job::SCHEMA_VERSION,
            directory: cache.display().to_string(),
            entries,
            total_bytes,
            removed_bytes,
        })
    });
    match done.and_then(|result| print(&result, json, out)) {
        Ok(()) => 0,
        Err(error) => report(
            err,
            Code::OutputFailed,
            format!("{}: {error}", cache.display()),
        ),
    }
}
