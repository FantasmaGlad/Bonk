//! Types communs aux opérations : résultats sérialisables, erreur, dépendances injectées.

use std::fmt::Display;
use std::path::{Path, PathBuf};

use bonk_core::analysis::ImageReport;
use bonk_core::codec::Format;
use bonk_core::format::Filesystem;
use bonk_core::format::table::{PartitionPlan, TableKind};
use bonk_core::repair::RepairReport;
use bonk_core::write::{VerifyReport, WriteReport};
use bonk_core::{BlockDevice, BlockError};
use bonk_hal::platform::Capability;
use bonk_hal::{Access, Disk, Elevation, EnumerateError, OpenTargetError};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;

/// Version des formats de résultat, incrémentée à chaque changement incompatible.
pub const SCHEMA_VERSION: u32 = 1;

/// Niveau de détail de `--trace-io`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceIo {
    /// Décalage, longueur, durée et somme de contrôle de chaque accès.
    Metadata,
    /// Avec les 16 premiers octets : réservé aux fichiers image.
    Data,
}

/// Que faire de l'analyse de l'image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteMode {
    /// Refuse une image qui ne démarrerait pas écrite telle quelle, ou tronquée.
    #[default]
    Auto,
    /// Écrit les octets tels quels, quelle que soit l'analyse.
    Raw,
    /// Copie les fichiers d'une ISO d'installation de Windows sur une partition FAT32.
    Extract,
}

/// Une erreur d'opération, avec son code stable.
#[derive(Debug)]
pub struct JobError {
    pub code: Code,
    pub message: String,
}

pub(super) fn fail(code: Code, message: impl Display) -> JobError {
    JobError {
        code,
        message: message.to_string(),
    }
}

/// Démontage des partitions d'un disque : rend les points démontés.
pub type UnmountFn = Box<dyn Fn(&Disk, Elevation) -> Result<Vec<String>, OpenTargetError>>;
/// Ouverture d'un disque entier en accès brut.
pub type OpenNodeFn =
    Box<dyn Fn(&Path, Access, Elevation) -> Result<Box<dyn BlockDevice>, BlockError>>;
/// Obtention du droit d'ouvrir un disque, avant tout geste sur lui.
pub type PrepareFn = Box<dyn Fn(&Path, Access, Elevation) -> Result<(), BlockError>>;

/// Formatage d'une partition par les outils de l'hôte.
pub type FormatPartitionFn =
    Box<dyn Fn(&Path, Filesystem, Option<&str>, Elevation) -> Result<(), BlockError>>;

/// Relecture de la table de partitions d'un disque par le système.
pub type RescanFn = Box<dyn Fn(&Path, Elevation) -> Result<(), BlockError>>;

/// Ce que les opérations demandent au système : injectable pour les tests.
pub struct Io {
    pub prepare: PrepareFn,
    pub unmount: UnmountFn,
    pub open_node: OpenNodeFn,
    pub cancelled: Box<dyn Fn() -> bool>,
    pub capabilities: Box<dyn Fn() -> Vec<Capability>>,
    pub format_partition: FormatPartitionFn,
    pub rescan: RescanFn,
    /// Attente courte : entre deux regards sur une partition que le système n'a pas encore créée, entre deux
    /// essais d'un téléchargement.
    pub pause: Box<dyn Fn()>,
}

impl Default for Io {
    fn default() -> Self {
        Self::with_cancel(None)
    }
}

impl Io {
    /// Accès système réels. `cancel` est un drapeau d'annulation propre à l'opération (le service en
    /// donne un par opération) ; sous Unix, Ctrl+C et SIGTERM annulent aussi les commandes et le
    /// service, qui appellent `interrupt::install`.
    pub fn with_cancel(cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>) -> Self {
        let cancelled = std::sync::Arc::new(move || {
            bonk_hal::interrupt::requested()
                || cancel
                    .as_ref()
                    .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
        });
        let (for_prepare, for_open) = (cancelled.clone(), cancelled.clone());
        Self {
            prepare: Box::new(move |path, access, elevation| {
                bonk_hal::platform::current().prepare_access(path, access, elevation, &*for_prepare)
            }),
            unmount: Box::new(|disk, elevation| {
                bonk_hal::platform::current().unmount(disk, elevation)
            }),
            open_node: Box::new(move |path, access, elevation| {
                bonk_hal::platform::current().open_disk(path, access, elevation, &*for_open)
            }),
            capabilities: Box::new(|| bonk_hal::platform::current().capabilities()),
            format_partition: Box::new(|path, fs, label, elevation| {
                bonk_hal::platform::current().format_partition(path, fs, label, elevation)
            }),
            rescan: Box::new(|path, elevation| {
                bonk_hal::platform::current().rescan(path, elevation)
            }),
            pause: Box::new(|| std::thread::sleep(std::time::Duration::from_millis(250))),
            cancelled: Box::new(move || cancelled()),
        }
    }
}

/// Dépendances d'une opération : énumération des disques et accès système.
pub struct Deps<'a> {
    pub enumerate: &'a dyn Fn() -> Result<Vec<Disk>, EnumerateError>,
    pub io: &'a Io,
}

/// Où écrire.
#[derive(Debug, Clone)]
pub enum Target {
    /// Un disque, par son identifiant stable (jamais `/dev/sdX`).
    Disk(String),
    /// Un fichier image (mode `--simulate`), créé à `size` octets s'il n'existe pas.
    Image { path: PathBuf, size: Option<u64> },
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TargetInfo {
    /// `disk` or `image`.
    pub kind: &'static str,
    pub id: Option<String>,
    pub path: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SourceInfo {
    pub path: String,
    pub format: Format,
    /// Size known in advance (raw image only).
    pub size_bytes: Option<u64>,
}

/// What a write will do, without doing it (`--dry-run`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct Plan {
    pub schema_version: u32,
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub image: ImageReport,
    /// The given hash was checked and matches.
    pub sha256_checked: bool,
    /// Mount points that will be unmounted.
    pub unmount: Vec<String>,
    pub steps: Vec<&'static str>,
    /// Windows drive: what it will receive. Null for a raw write.
    pub extraction: Option<super::extract::Extraction>,
}

/// Result of a write.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub image: ImageReport,
    /// The given hash was checked and matches.
    pub sha256_checked: bool,
    pub unmounted: Vec<String>,
    pub write: WriteReport,
    pub verify: Option<VerifyReport>,
    pub gpt_repair: Option<RepairReport>,
}

/// Result of a verification without writing.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VerifyDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub verify: VerifyReport,
    /// Bytes excluded from the comparison (`sha256` is then the image hash with these bytes zeroed). Empty for an image without GPT or as large as the drive.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_ranges: Vec<IgnoredRange>,
}

/// Byte range excluded from a comparison, with the reason.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IgnoredRange {
    pub offset: u64,
    pub length: u64,
    /// `gpt_repair`: fields the GPT repair rewrites on a drive larger than the image.
    pub reason: &'static str,
}

/// Result (or plan) of a GPT repair.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RepairDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub needs_repair: bool,
    /// Codes of the findings before and after.
    pub findings_before: Vec<String>,
    pub repair: Option<RepairReport>,
    pub findings_after: Option<Vec<String>>,
}

/// What a format will do, without doing it (`--dry-run`), or what it did.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FormatDone {
    pub schema_version: u32,
    /// `plan` or `result`.
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub table: TableKind,
    pub filesystem: Filesystem,
    /// The filesystem was chosen by default, from the size of the drive.
    pub filesystem_defaulted: bool,
    /// Maximum file size on this filesystem, when it can get in the way; `--fs exfat` lifts it.
    pub file_size_limit_bytes: Option<u64>,
    pub label: Option<String>,
    pub partition: PartitionPlan,
    /// Written by Bonk (`bonk`) or by host tools (`system`).
    pub written_by: &'static str,
    pub cluster_bytes: Option<u32>,
    /// Mount points unmounted, or that would be.
    pub unmounted: Vec<String>,
    /// Table and structures read back after writing.
    pub verified: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum WriteOutcome {
    Plan(Box<Plan>),
    Done(Box<WriteDone>),
    Extracted(Box<super::extract::ExtractDone>),
}
