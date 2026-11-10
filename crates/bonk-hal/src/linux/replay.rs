use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};

use super::sources::io_error;
use super::{LinuxRoots, enumerate};

/// Rebuilds the tree the reads saw (files, `/sys/block` links) in a temporary directory, enumerates it, then
/// removes it.
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

/// A capture made under another root (tests) keeps its prefix, removed from `/sys/`, `/proc/` or the udev
/// database on.
pub(super) fn standard_path(path: &str) -> &str {
    ["/sys/", "/proc/", crate::enumerate::UDEV_DATA]
        .iter()
        .filter_map(|anchor| path.find(anchor))
        .min()
        .map_or(path, |start| &path[start..])
}

/// A capture is a file from someone else: none of its paths may lead out of the rebuilt tree.
fn relative(path: &str) -> io::Result<&Path> {
    let path = Path::new(path.trim_start_matches('/'));
    if path
        .components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("capture path leaves its tree: {}", path.display()),
        ))
    }
}

/// One file name, as listed in `holders` or under `/sys/block`.
fn file_name(name: &str) -> io::Result<&str> {
    let path = relative(name)?;
    if path.components().count() == 1 {
        Ok(name)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("capture name is not a file name: {name}"),
        ))
    }
}

pub(super) fn build_tree(base: &Path, reads: &[SourceRead]) -> io::Result<()> {
    use std::collections::BTreeMap;
    let udev = crate::enumerate::UDEV_DATA.trim_start_matches('/');
    for dir in ["sys/block", "proc/self", udev, "dev"] {
        fs::create_dir_all(base.join(dir))?;
    }
    // `/sys/block/<name>` links to the canonical device path.
    let links: BTreeMap<&str, &str> = reads
        .iter()
        .filter(|r| r.kind == "link")
        .filter_map(|r| r.content.as_deref().map(|c| (standard_path(&r.path), c)))
        .collect();
    for (link, relative) in &links {
        let Some(name) = link.rsplit('/').next() else {
            continue;
        };
        let name = file_name(name)?;
        fs::create_dir_all(base.join("sys").join(self::relative(relative)?))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(format!("../{relative}"), base.join("sys/block").join(name))?;
    }
    // A path read through a link is written at the canonical place.
    let place = |path: &str| -> io::Result<PathBuf> {
        let path = standard_path(path);
        for (link, relative) in &links {
            if let Some(rest) = path
                .strip_prefix(link)
                .filter(|r| r.is_empty() || r.starts_with('/'))
            {
                return Ok(base
                    .join("sys")
                    .join(self::relative(relative)?)
                    .join(self::relative(rest)?));
            }
        }
        Ok(base.join(self::relative(path)?))
    };
    for read in reads.iter().filter(|r| r.ok) {
        match read.kind.as_str() {
            "file" => {
                let target = place(&read.path)?;
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(target, read.content.as_deref().unwrap_or_default())?;
            }
            "dir" => {
                let target = place(&read.path)?;
                fs::create_dir_all(&target)?;
                // Stacked devices are only known by name.
                if standard_path(&read.path).ends_with("/holders") {
                    for name in read.content.as_deref().unwrap_or_default().lines() {
                        fs::write(target.join(file_name(name)?), "")?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}
