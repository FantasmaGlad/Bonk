//! macOS : collecte des faits sur les disques par `diskutil`, en lecture seule.
//!
//! `diskutil` est présent sur tout macOS et ne demande aucun droit pour lire ; passer par lui évite une
//! dépendance et du code `unsafe`. `plutil` convertit sa sortie en JSON. L'interprétation vit dans
//! [`crate::macos_facts`], testée sur tous les systèmes.

use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Instant;

use bonk_core::format::Filesystem;
use bonk_core::{BlockDevice, BlockError};
use serde_json::Value;

use super::{Capability, Elevation, Platform, Unsupported};
use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::macos_facts::{RawMedia, SOURCE, media_from_info, normalize};
use crate::target::OpenTargetError;

/// Les fonctions d'écriture restent celles d'un système sans accès brut jusqu'à leur arrivée.
#[derive(Debug, Clone, Copy, Default)]
pub struct Macos;

fn failed(what: &str, source: io::Error) -> EnumerateError {
    EnumerateError::Io {
        what: what.to_owned(),
        source,
    }
}

/// Lance `diskutil <arguments> -plist` et rend sa sortie en JSON. Un outil absent ou en échec fait échouer
/// l'énumération : proposer des disques sans ces faits serait deviner.
fn diskutil(arguments: &[&str]) -> Result<Value, EnumerateError> {
    let what = format!("diskutil {}", arguments.join(" "));
    let output = Command::new("/usr/sbin/diskutil")
        .args(arguments)
        .output()
        .map_err(|e| failed(&what, e))?;
    if !output.status.success() {
        return Err(failed(
            &what,
            io::Error::other(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        ));
    }
    let mut child = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| failed("plutil", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&output.stdout)
            .map_err(|e| failed("plutil", e))?;
    }
    let converted = child.wait_with_output().map_err(|e| failed("plutil", e))?;
    if !converted.status.success() {
        return Err(failed(
            "plutil",
            io::Error::other(String::from_utf8_lossy(&converted.stderr).trim().to_owned()),
        ));
    }
    serde_json::from_slice(&converted.stdout).map_err(|e| EnumerateError::Malformed {
        what,
        detail: e.to_string(),
    })
}

fn collect() -> Result<Vec<RawMedia>, EnumerateError> {
    let list = diskutil(&["list", "-plist"])?;
    let devices = list["AllDisks"]
        .as_array()
        .ok_or_else(|| EnumerateError::Malformed {
            what: "diskutil list".to_owned(),
            detail: "no AllDisks".to_owned(),
        })?;
    devices
        .iter()
        .filter_map(Value::as_str)
        .map(|device| media_from_info(&diskutil(&["info", "-plist", device])?))
        .collect()
}

impl Platform for Macos {
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError> {
        normalize(&collect()?)
    }

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
        let started = Instant::now();
        let media = collect();
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        let mut read = SourceRead {
            path: SOURCE.to_owned(),
            kind: "file".to_owned(),
            ok: media.is_ok(),
            bytes: None,
            error: None,
            micros,
            content: None,
        };
        match media {
            Ok(media) => {
                let content = serde_json::to_string(&media).unwrap_or_default();
                read.bytes = Some(content.len());
                read.content = Some(content);
                (normalize(&media), vec![read])
            }
            Err(error) => {
                read.error = Some(error.to_string());
                (Err(error), vec![read])
            }
        }
    }

    fn enumerate_from_capture(&self, _reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
        Err(EnumerateError::Malformed {
            what: "capture".to_owned(),
            detail: "no macOS facts in this capture".to_owned(),
        })
    }

    fn unmount(&self, disk: &Disk, elevation: Elevation) -> Result<Vec<String>, OpenTargetError> {
        Unsupported.unmount(disk, elevation)
    }

    fn prepare_access(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), BlockError> {
        Unsupported.prepare_access(path, access, elevation, cancelled)
    }

    fn open_disk(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn BlockDevice>, BlockError> {
        Unsupported.open_disk(path, access, elevation, cancelled)
    }

    fn capabilities(&self) -> Vec<Capability> {
        Unsupported.capabilities()
    }

    fn format_partition(
        &self,
        partition: &Path,
        fs: Filesystem,
        label: Option<&str>,
        elevation: Elevation,
    ) -> Result<(), BlockError> {
        Unsupported.format_partition(partition, fs, label, elevation)
    }

    fn rescan(&self, disk: &Path, elevation: Elevation) -> Result<(), BlockError> {
        Unsupported.rescan(disk, elevation)
    }

    fn is_privileged(&self) -> bool {
        // SAFETY : `geteuid` n'a ni argument ni effet de bord.
        unsafe { libc::geteuid() == 0 }
    }

    fn kernel_release(&self) -> Option<String> {
        None
    }
}
