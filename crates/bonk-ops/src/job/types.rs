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
    /// Attente courte entre deux regards sur une partition que le système n'a pas encore créée.
    pub pause: Box<dyn Fn()>,
}

impl Default for Io {
    fn default() -> Self {
        Self::with_cancel(None)
    }
}

impl Io {
    /// Accès système réels. `cancel` est un drapeau d'annulation propre à l'opération (le service en
    /// donne un par opération) ; Ctrl+C annule aussi les commandes, qui appellent `interrupt::install`.
    pub fn with_cancel(cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>) -> Self {
        Self {
            prepare: Box::new(|path, access, elevation| {
                bonk_hal::platform::current().prepare_access(path, access, elevation)
            }),
            unmount: Box::new(|disk, elevation| {
                bonk_hal::platform::current().unmount(disk, elevation)
            }),
            open_node: Box::new(|path, access, elevation| {
                bonk_hal::platform::current().open_disk(path, access, elevation)
            }),
            capabilities: Box::new(|| bonk_hal::platform::current().capabilities()),
            format_partition: Box::new(|path, fs, label, elevation| {
                bonk_hal::platform::current().format_partition(path, fs, label, elevation)
            }),
            rescan: Box::new(|path, elevation| {
                bonk_hal::platform::current().rescan(path, elevation)
            }),
            pause: Box::new(|| std::thread::sleep(std::time::Duration::from_millis(250))),
            cancelled: Box::new(move || {
                bonk_hal::interrupt::requested()
                    || cancel
                        .as_ref()
                        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
            }),
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
    /// `disk` ou `image`.
    pub kind: &'static str,
    pub id: Option<String>,
    pub path: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SourceInfo {
    pub path: String,
    pub format: Format,
    /// Taille connue d'avance (image brute seulement).
    pub size_bytes: Option<u64>,
}

/// Ce que fera une écriture, sans rien faire (`--dry-run`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct Plan {
    pub schema_version: u32,
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub image: ImageReport,
    /// L'empreinte fournie a été contrôlée et correspond.
    pub sha256_checked: bool,
    /// Points de montage qui seront démontés.
    pub unmount: Vec<String>,
    pub steps: Vec<&'static str>,
}

/// Résultat d'une écriture.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub image: ImageReport,
    /// L'empreinte fournie a été contrôlée et correspond.
    pub sha256_checked: bool,
    pub unmounted: Vec<String>,
    pub write: WriteReport,
    pub verify: Option<VerifyReport>,
    pub gpt_repair: Option<RepairReport>,
}

/// Résultat d'une vérification sans écriture.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VerifyDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub verify: VerifyReport,
    /// Octets exclus de la comparaison (`sha256` est alors l'empreinte de l'image avec ces octets remis
    /// à zéro). Vide pour une image sans GPT ou aussi grande que le support.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_ranges: Vec<IgnoredRange>,
}

/// Plage d'octets exclue d'une comparaison, avec la raison.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IgnoredRange {
    pub offset: u64,
    pub length: u64,
    /// `gpt_repair` : champs que la réparation du GPT réécrit sur un support plus grand que l'image.
    pub reason: &'static str,
}

/// Résultat (ou plan) d'une réparation du GPT.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RepairDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub needs_repair: bool,
    /// Codes des constats avant et après.
    pub findings_before: Vec<String>,
    pub repair: Option<RepairReport>,
    pub findings_after: Option<Vec<String>>,
}

/// Ce que fera un formatage, sans rien faire (`--dry-run`), ou ce qu'il a fait.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FormatDone {
    pub schema_version: u32,
    /// `plan` ou `result`.
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub table: TableKind,
    pub filesystem: Filesystem,
    /// Le système de fichiers a été choisi par défaut, d'après la taille du support.
    pub filesystem_defaulted: bool,
    pub label: Option<String>,
    pub partition: PartitionPlan,
    /// Écrit par Bonk (`bonk`) ou par les outils de l'hôte (`system`).
    pub written_by: &'static str,
    pub cluster_bytes: Option<u32>,
    /// Points de montage démontés, ou qui le seraient.
    pub unmounted: Vec<String>,
    /// Table et structures relues après écriture.
    pub verified: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum WriteOutcome {
    Plan(Box<Plan>),
    Done(Box<WriteDone>),
}
