//! `bonk cache` : les images téléchargées restent pour éviter de les retélécharger ; rien ne les supprime sans
//! qu'on le demande. Seul ce que Bonk a rangé (`images/<empreinte>/`) se liste et se vide.

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

/// Où Bonk garde ses téléchargements : le dossier donné, sinon le dossier de cache de l'utilisateur.
pub(crate) fn directory(given: Option<&Path>) -> Option<PathBuf> {
    given
        .map(Path::to_path_buf)
        .or_else(|| bonk_hal::platform::cache_dir(&|name| std::env::var(name).ok()))
}

/// Dossier d'une image dans le cache : rangée sous son empreinte, une nouvelle version de même nom ne reprend
/// jamais l'ancienne.
pub(crate) fn image_path(cache: &Path, sha256: &str, name: &str) -> PathBuf {
    cache.join(IMAGES).join(sha256).join(name)
}

/// Les images téléchargées
#[derive(Debug, Subcommand)]
pub(crate) enum CacheAction {
    /// List the kept images, incomplete downloads included
    List(ListArgs),
    /// Remove the kept images; an address is downloaded again on the next write
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

fn is_sha256(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Dossiers d'empreintes réels, sans suivre de lien : un lien posé là ne mène jamais une suppression ailleurs.
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
        if is_sha256(&name) && entry.file_type()?.is_dir() {
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
            entries.push(Entry {
                partial: name.ends_with(".part"),
                bytes: file.metadata()?.len(),
                name,
                sha256: sha256.clone(),
            });
        }
    }
    Ok(entries)
}

/// Taille de ce que le cache garde.
pub(crate) fn total_bytes(cache: &Path) -> io::Result<u64> {
    Ok(read_entries(cache)?.iter().map(|entry| entry.bytes).sum())
}

fn remove_files(cache: &Path) -> io::Result<()> {
    for (_, directory) in hash_directories(cache)? {
        fs::remove_dir_all(directory)?;
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
    let Some(cache) = directory(given) else {
        return report(
            err,
            Code::InvalidArgument,
            "no known cache directory: give it with --cache-dir",
        );
    };
    let done = read_entries(&cache).and_then(|entries| {
        let total_bytes = entries.iter().map(|entry| entry.bytes).sum();
        let removed_bytes = match clean {
            Some(false) => {
                remove_files(&cache)?;
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
