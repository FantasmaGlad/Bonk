//! Paramètres des méthodes, validés avant tout geste.

use std::path::PathBuf;

use bonk_core::format::Filesystem;
use bonk_core::format::table::TableKind;
use bonk_hal::{Elevation, FilterOptions};
use serde::Deserialize;

use crate::config::ViewDefaults;
use crate::errors::Code;
use bonk_ops::job::{Target, TraceIo, WriteMode};

use super::protocol::{Failure, failure};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct ViewParams {
    pub(super) all: bool,
    pub(super) show_internal: bool,
    pub(super) allow_virtual: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TargetParams {
    pub(super) target: String,
    #[serde(default)]
    pub(super) show_internal: bool,
    #[serde(default)]
    pub(super) allow_virtual: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct DiagnoseParams {
    pub(super) include_serials: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct NameParams {
    pub(super) code: Option<String>,
    pub(super) name: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct ConfigParams {
    pub(super) key: Option<String>,
    pub(super) value: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct WatchParams {
    pub(super) interval_ms: Option<u64>,
    pub(super) all: bool,
    pub(super) show_internal: bool,
    pub(super) allow_virtual: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub(super) struct TransferParams {
    pub(super) target: String,
    pub(super) source: Option<PathBuf>,
    /// La cible est un fichier image (créé avec `size` s'il n'existe pas).
    pub(super) simulate: bool,
    pub(super) size: Option<u64>,
    pub(super) yes: bool,
    pub(super) dry_run: bool,
    pub(super) no_verify: bool,
    pub(super) no_repair_gpt: bool,
    pub(super) show_internal: bool,
    pub(super) allow_virtual: bool,
    /// `metadata` ou `data`.
    pub(super) trace_io: Option<String>,
    /// Empreinte SHA-256 publiée pour le fichier source.
    pub(super) sha256: Option<String>,
    /// `auto` (par défaut) ou `raw`.
    pub(super) mode: Option<String>,
    /// `never` (par défaut : personne devant l'écran) ou `auto`.
    pub(super) elevate: Option<String>,
    /// Tient lieu de `yes` et autorise un disque interne admis par `show_internal`.
    pub(super) force: bool,
    /// Système de fichiers de `disk.format` : `fat32`, `exfat`, `ntfs`, `ext4`.
    pub(super) fs: Option<String>,
    /// Table de `disk.format` : `gpt` (par défaut) ou `mbr`.
    pub(super) table: Option<String>,
    /// Étiquette de `disk.format`.
    pub(super) label: Option<String>,
}

impl TransferParams {
    pub(super) fn options(&self, view: &ViewDefaults) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal || view.show_internal,
            allow_virtual: self.allow_virtual || view.allow_virtual,
        }
    }

    /// Pour écrire, seule la requête compte : un réglage n'autorise jamais à effacer un disque de plus.
    pub(super) fn write_options(&self) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
        }
    }

    pub(super) fn target(&self) -> Result<Target, Failure> {
        if self.target.is_empty() {
            return Err(failure(Code::ProtocolError, "paramètre « target » requis"));
        }
        Ok(if self.simulate {
            Target::Image {
                path: PathBuf::from(&self.target),
                size: self.size,
            }
        } else {
            Target::Disk(self.target.clone())
        })
    }

    pub(super) fn trace(&self) -> Result<Option<TraceIo>, Failure> {
        match self.trace_io.as_deref() {
            None => Ok(None),
            Some("metadata") => Ok(Some(TraceIo::Metadata)),
            Some("data") => Ok(Some(TraceIo::Data)),
            Some(autre) => Err(failure(
                Code::ProtocolError,
                format!("trace_io inconnu : « {autre} » (metadata ou data)"),
            )),
        }
    }

    pub(super) fn mode(&self) -> Result<WriteMode, Failure> {
        match self.mode.as_deref() {
            None | Some("auto") => Ok(WriteMode::Auto),
            Some("raw") => Ok(WriteMode::Raw),
            Some(autre) => Err(failure(
                Code::ProtocolError,
                format!("mode inconnu : « {autre} » (auto ou raw)"),
            )),
        }
    }

    pub(super) fn elevation(&self) -> Result<Elevation, Failure> {
        match self.elevate.as_deref() {
            None | Some("never") => Ok(Elevation::Never),
            Some("auto") => Ok(Elevation::Auto),
            Some(autre) => Err(failure(
                Code::ProtocolError,
                format!("elevate inconnu : « {autre} » (auto ou never)"),
            )),
        }
    }

    pub(super) fn filesystem(&self) -> Result<Option<Filesystem>, Failure> {
        match self.fs.as_deref() {
            None => Ok(None),
            Some(name) => Filesystem::ALL
                .into_iter()
                .find(|fs| fs.name() == name)
                .map(Some)
                .ok_or_else(|| {
                    failure(
                        Code::ProtocolError,
                        format!("fs inconnu : « {name} » (fat32, exfat, ntfs ou ext4)"),
                    )
                }),
        }
    }

    pub(super) fn table(&self) -> Result<TableKind, Failure> {
        match self.table.as_deref() {
            None => Ok(TableKind::Gpt),
            Some(name) => TableKind::ALL
                .into_iter()
                .find(|table| table.name() == name)
                .ok_or_else(|| {
                    failure(
                        Code::ProtocolError,
                        format!("table inconnue : « {name} » (gpt ou mbr)"),
                    )
                }),
        }
    }

    pub(super) fn source(&self) -> Result<PathBuf, Failure> {
        self.source
            .clone()
            .ok_or_else(|| failure(Code::ProtocolError, "paramètre « source » requis"))
    }
}
