//! Lecture des sources système : chaque lecture est journalisée au niveau `trace` et, pour
//! `bonk debug sources`, enregistrée avec son contenu.

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};

use super::{LinuxRoots, enumerate};

/// Taille maximale du contenu conservé par lecture enregistrée.
pub(super) const CONTENT_LIMIT: usize = 8 * 1024;

thread_local! {
    // Enregistrement propre au fil d'exécution qui énumère : la fenêtre énumère sur un autre fil et
    // n'est donc jamais gênée par un `debug sources` en cours.
    static RECORDER: RefCell<Option<Vec<SourceRead>>> = const { RefCell::new(None) };
}

pub(super) fn note_read(
    kind: &'static str,
    path: &Path,
    started: Instant,
    outcome: Result<(usize, &str), &io::Error>,
) {
    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let shown = path.display().to_string();
    match &outcome {
        Ok((bytes, _)) => log::trace!(
            target: "bonk_hal::linux",
            path = shown.as_str(), kind = kind, bytes = *bytes, micros = micros;
            "read source"
        ),
        Err(err) => log::trace!(
            target: "bonk_hal::linux",
            path = shown.as_str(), kind = kind, error = err.to_string().as_str(), micros = micros;
            "read source failed"
        ),
    }
    RECORDER.with(|recorder| {
        if let Some(reads) = recorder.borrow_mut().as_mut() {
            let (bytes, error, content) = match outcome {
                Ok((bytes, content)) => {
                    let mut kept = content.to_owned();
                    if kept.len() > CONTENT_LIMIT {
                        let mut end = CONTENT_LIMIT;
                        while !kept.is_char_boundary(end) {
                            end -= 1;
                        }
                        kept.truncate(end);
                    }
                    (Some(bytes), None, Some(kept))
                }
                Err(err) => (None, Some(err.to_string()), None),
            };
            reads.push(SourceRead {
                path: shown,
                kind: kind.to_owned(),
                ok: error.is_none(),
                bytes,
                error,
                micros,
                content,
            });
        }
    });
}

/// Enregistre qu'un chemin de `/sys/block` est un lien vers `relative` (chemin canonique du
/// périphérique, d'où se déduit le bus) : sans lui, une capture rejouée perdrait le bus.
pub(super) fn note_link(path: &Path, relative: &Path) {
    let target = relative.display().to_string();
    log::trace!(target: "bonk_hal::linux", path = path.display().to_string().as_str(), target = target.as_str(); "resolved link");
    RECORDER.with(|recorder| {
        if let Some(reads) = recorder.borrow_mut().as_mut() {
            reads.push(SourceRead {
                path: path.display().to_string(),
                kind: "link".to_owned(),
                ok: true,
                bytes: Some(target.len()),
                error: None,
                micros: 0,
                content: Some(target),
            });
        }
    });
}

pub(super) fn traced_read(path: &Path) -> io::Result<String> {
    let started = Instant::now();
    let result = fs::read_to_string(path);
    note_read(
        "file",
        path,
        started,
        result.as_ref().map(|text| (text.len(), text.as_str())),
    );
    result
}

pub(super) fn traced_list(path: &Path) -> io::Result<Vec<String>> {
    let started = Instant::now();
    let result = (|| {
        let mut names = Vec::new();
        for entry in fs::read_dir(path)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    })();
    let joined = result.as_ref().map(|names: &Vec<String>| names.join("\n"));
    note_read(
        "dir",
        path,
        started,
        joined
            .as_ref()
            .map_err(|erreur| *erreur)
            .map(|text| (text.len(), text.as_str())),
    );
    result
}

/// Énumère comme [`enumerate`] et rend aussi toutes les lectures de sources, pour `bonk debug sources`.
pub fn enumerate_recorded(
    roots: &LinuxRoots,
) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
    RECORDER.with(|recorder| *recorder.borrow_mut() = Some(Vec::new()));
    let result = enumerate(roots);
    let reads = RECORDER.with(|recorder| recorder.borrow_mut().take().unwrap_or_default());
    (result, reads)
}

pub(super) fn io_error(path: &Path, source: io::Error) -> EnumerateError {
    EnumerateError::Io {
        what: path.display().to_string(),
        source,
    }
}

pub(super) fn read_required(path: &Path) -> Result<String, EnumerateError> {
    traced_read(path).map_err(|e| io_error(path, e))
}

/// Attribut descriptif : absent ou illisible vaut « inconnu ».
pub(super) fn read_optional(path: &Path) -> Option<String> {
    let text = traced_read(path).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub(super) fn canonical(path: &Path) -> Result<PathBuf, EnumerateError> {
    fs::canonicalize(path).map_err(|e| io_error(path, e))
}

pub(super) fn list_dir(path: &Path) -> Result<Vec<String>, EnumerateError> {
    traced_list(path).map_err(|e| io_error(path, e))
}

/// Comme [`list_dir`], mais un répertoire absent est vide (`holders` n'existe que pour les disques
/// que le noyau a rendus visibles).
pub(super) fn list_dir_optional(path: &Path) -> Result<Vec<String>, EnumerateError> {
    match traced_list(path) {
        Ok(mut names) => {
            names.sort();
            Ok(names)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(io_error(path, err)),
    }
}

pub(super) fn read_devnum(dir: &Path) -> Result<String, EnumerateError> {
    Ok(read_required(&dir.join("dev"))?.trim().to_owned())
}

pub(super) fn read_size(dir: &Path) -> Result<u64, EnumerateError> {
    let path = dir.join("size");
    let text = read_required(&path)?;
    let sectors: u64 = text.trim().parse().map_err(|_| EnumerateError::Malformed {
        what: path.display().to_string(),
        detail: format!("taille illisible : {:?}", text.trim()),
    })?;
    // Le noyau exprime toujours `size` en secteurs de 512 octets, quelle que soit la taille logique.
    sectors
        .checked_mul(512)
        .ok_or_else(|| EnumerateError::Malformed {
            what: path.display().to_string(),
            detail: "taille hors limites".to_owned(),
        })
}
