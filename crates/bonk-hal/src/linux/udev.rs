//! Base udev : signatures, numéros de série, types de partition et liens `by-id`.

use std::collections::BTreeMap;
use std::io;

use crate::enumerate::EnumerateError;

use super::LinuxRoots;
use super::sources::{io_error, traced_read};

#[derive(Debug, Default)]
pub(super) struct UdevRecord {
    pub(super) symlinks: Vec<String>,
    pub(super) props: BTreeMap<String, String>,
}

pub(super) fn parse_udev(text: &str) -> UdevRecord {
    let mut record = UdevRecord::default();
    for line in text.lines() {
        if let Some(link) = line.strip_prefix("S:") {
            record.symlinks.push(link.to_owned());
        } else if let Some((key, value)) = line.strip_prefix("E:").and_then(|e| e.split_once('=')) {
            record.props.insert(key.to_owned(), value.to_owned());
        }
    }
    record
}

/// Enregistrement udev d'un périphérique ; vide s'il n'en a pas (le répertoire existe : voir
/// [`enumerate`]).
pub(super) fn load_udev(roots: &LinuxRoots, devnum: &str) -> Result<UdevRecord, EnumerateError> {
    let path = roots.udev_data.join(format!("b{devnum}"));
    match traced_read(&path) {
        Ok(text) => Ok(parse_udev(&text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(UdevRecord::default()),
        Err(err) => Err(io_error(&path, err)),
    }
}

impl UdevRecord {
    pub(super) fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}
