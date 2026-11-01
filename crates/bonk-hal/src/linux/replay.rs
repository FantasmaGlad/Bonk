//! Rejeu d'une capture : l'arborescence lue est reconstruite sous un dossier temporaire.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};

use super::sources::io_error;
use super::{LinuxRoots, enumerate};

/// Rejoue une capture : reconstruit sous un dossier temporaire l'arborescence que les lectures ont
/// vue (fichiers, liens de `/sys/block`), énumère dessus, puis la supprime.
pub(crate) fn enumerate_from_reads(reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!("bonk-fixture-{}-{n}", std::process::id()));
    let _cleanup = Cleanup(base.clone());
    build_tree(&base, reads).map_err(|e| io_error(&base, e))?;
    enumerate(&LinuxRoots::under(&base))
}

/// Chemin tel qu'il est sur un vrai système : une capture faite sous une autre racine (les tests)
/// garde son préfixe, qu'on retire en repartant de `/sys/`, `/proc/` ou `/run/udev/`.
pub(super) fn standard_path(path: &str) -> &str {
    ["/sys/", "/proc/", "/run/udev/"]
        .iter()
        .filter_map(|anchor| path.find(anchor))
        .min()
        .map_or(path, |start| &path[start..])
}

pub(super) fn build_tree(base: &Path, reads: &[SourceRead]) -> io::Result<()> {
    use std::collections::BTreeMap;
    for dir in ["sys/block", "proc/self", "run/udev/data", "dev"] {
        fs::create_dir_all(base.join(dir))?;
    }
    // `/sys/block/<nom>` : lien vers le chemin canonique du périphérique.
    let links: BTreeMap<&str, &str> = reads
        .iter()
        .filter(|r| r.kind == "link")
        .filter_map(|r| r.content.as_deref().map(|c| (standard_path(&r.path), c)))
        .collect();
    for (link, relative) in &links {
        let Some(name) = link.rsplit('/').next() else {
            continue;
        };
        fs::create_dir_all(base.join("sys").join(relative))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(format!("../{relative}"), base.join("sys/block").join(name))?;
    }
    // Un chemin lu à travers un lien est écrit à l'emplacement canonique.
    let place = |path: &str| -> PathBuf {
        let path = standard_path(path);
        for (link, relative) in &links {
            if let Some(rest) = path
                .strip_prefix(link)
                .filter(|r| r.is_empty() || r.starts_with('/'))
            {
                return base
                    .join("sys")
                    .join(relative)
                    .join(rest.trim_start_matches('/'));
            }
        }
        base.join(path.trim_start_matches('/'))
    };
    for read in reads.iter().filter(|r| r.ok) {
        match read.kind.as_str() {
            "file" => {
                let target = place(&read.path);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, read.content.as_deref().unwrap_or_default())?;
            }
            "dir" => {
                let target = place(&read.path);
                fs::create_dir_all(&target)?;
                // Les périphériques empilés ne sont connus que par leurs noms.
                if standard_path(&read.path).ends_with("/holders") {
                    for name in read.content.as_deref().unwrap_or_default().lines() {
                        fs::write(target.join(name), "")?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
