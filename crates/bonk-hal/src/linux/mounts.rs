//! Points de montage (`/proc/self/mountinfo`) et espaces d'échange actifs (`/proc/swaps`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use crate::enumerate::EnumerateError;

use super::LinuxRoots;
use super::sources::read_required;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct MountEntry {
    pub(super) devnum: String,
    pub(super) mount_point: String,
    pub(super) source: String,
}

/// Lit `/proc/self/mountinfo`. Une ligne illisible fait échouer l'énumération : mieux vaut ne rien
/// proposer qu'ignorer un montage.
pub(super) fn parse_mountinfo(text: &str) -> Result<Vec<MountEntry>, EnumerateError> {
    let malformed = |line: &str| EnumerateError::Malformed {
        what: "mountinfo".to_owned(),
        detail: format!("ligne illisible : {line:?}"),
    };
    let mut entries = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        // Un point de montage contenant « - » est échappé, donc le premier « - » isolé est bien le
        // séparateur entre les champs fixes et les champs de type de système de fichiers.
        let (before, after) = line.split_once(" - ").ok_or_else(|| malformed(line))?;
        let fields: Vec<&str> = before.split_whitespace().collect();
        let tail: Vec<&str> = after.split_whitespace().collect();
        if fields.len() < 5 || tail.len() < 2 {
            return Err(malformed(line));
        }
        entries.push(MountEntry {
            devnum: fields[2].to_owned(),
            mount_point: unescape_octal(fields[4]),
            source: unescape_octal(tail[1]),
        });
    }
    Ok(entries)
}

/// Défait l'échappement du noyau : `\040` pour l'espace, `\011` la tabulation, `\134` l'antislash...
pub(super) fn unescape_octal(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 4 <= bytes.len())
            .then(|| &bytes[i + 1..i + 4])
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)))
            .map(|d| d.iter().fold(0u32, |acc, b| acc * 8 + u32::from(b - b'0')))
            .and_then(|v| u8::try_from(v).ok());
        if let Some(byte) = octal {
            out.push(byte);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Montages indexés par numéro de périphérique **et** par nom.
///
/// Le numéro seul ne suffit pas : btrfs rapporte un numéro anonyme (`0:50`) et seule la source
/// (`/dev/sdc1`) désigne le vrai périphérique.
pub(super) struct MountIndex {
    pub(super) by_devnum: BTreeMap<String, Vec<String>>,
    pub(super) by_name: BTreeMap<String, Vec<String>>,
}

/// Nom du périphérique (`sda1`) derrière une source de montage (`/dev/sda1`, `/dev/disk/by-uuid/...`).
pub(super) fn device_name(roots: &LinuxRoots, source: &str) -> Option<String> {
    let relative = source.strip_prefix("/dev/")?;
    let path = roots.dev.join(relative);
    let resolved = fs::canonicalize(&path).unwrap_or(path);
    resolved.file_name()?.to_str().map(str::to_owned)
}

pub(super) fn active_swaps(roots: &LinuxRoots) -> Result<BTreeSet<String>, EnumerateError> {
    let text = read_required(&roots.proc.join("swaps"))?;
    Ok(text
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|source| device_name(roots, &unescape_octal(source)))
        .collect())
}

impl MountIndex {
    pub(super) fn new(roots: &LinuxRoots, mounts: &[MountEntry]) -> Self {
        let mut by_devnum: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for mount in mounts {
            by_devnum
                .entry(mount.devnum.clone())
                .or_default()
                .push(mount.mount_point.clone());
            if let Some(name) = device_name(roots, &mount.source) {
                by_name
                    .entry(name)
                    .or_default()
                    .push(mount.mount_point.clone());
            }
        }
        Self { by_devnum, by_name }
    }

    pub(super) fn of(&self, devnum: &str, name: &str) -> Vec<String> {
        let mut all: Vec<String> = self
            .by_devnum
            .get(devnum)
            .into_iter()
            .chain(self.by_name.get(name))
            .flatten()
            .cloned()
            .collect();
        all.sort();
        all.dedup();
        all
    }
}
