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

/// Result format version, bumped at each incompatible change.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceIo {
    /// Offset, length, duration and checksum of each access.
    Metadata,
    /// Plus the first 16 bytes: image files only.
    Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteMode {
    /// Refuses an image that would not boot written as is, or a truncated one.
    #[default]
    Auto,
    /// Writes the bytes as is, whatever the analysis says.
    Raw,
    /// Copies the files of a Windows installation ISO to a FAT32 partition.
    Extract,
}

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

/// Returns the unmounted points.
pub type UnmountFn = Box<dyn Fn(&Disk, Elevation) -> Result<Vec<String>, OpenTargetError>>;
pub type OpenNodeFn =
    Box<dyn Fn(&Path, Access, Elevation) -> Result<Box<dyn BlockDevice>, BlockError>>;
/// Before any action on the disk.
pub type PrepareFn = Box<dyn Fn(&Path, Access, Elevation) -> Result<(), BlockError>>;

pub type FormatPartitionFn =
    Box<dyn Fn(&Path, Filesystem, Option<&str>, Elevation) -> Result<(), BlockError>>;

pub type RescanFn = Box<dyn Fn(&Path, Elevation) -> Result<(), BlockError>>;

/// What operations ask of the system: injectable for tests.
pub struct Io {
    pub prepare: PrepareFn,
    pub unmount: UnmountFn,
    pub open_node: OpenNodeFn,
    pub cancelled: Box<dyn Fn() -> bool>,
    pub capabilities: Box<dyn Fn() -> Vec<Capability>>,
    pub format_partition: FormatPartitionFn,
    pub rescan: RescanFn,
    /// Short wait between looks at a partition the system has not created yet, and between download attempts.
    pub pause: Box<dyn Fn()>,
}

impl Default for Io {
    fn default() -> Self {
        Self::with_cancel(None)
    }
}

impl Io {
    /// Real system access. `cancel` is a per-operation cancel flag (the service gives one per operation); on
    /// Unix, Ctrl+C and SIGTERM also cancel commands and the service, which call `interrupt::install`.
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

pub struct Deps<'a> {
    pub enumerate: &'a dyn Fn() -> Result<Vec<Disk>, EnumerateError>,
    pub io: &'a Io,
}

#[derive(Debug, Clone)]
pub enum Target {
    /// By stable id, never `/dev/sdX`.
    Disk(String),
    /// `--simulate`: created at `size` bytes if missing.
    Image { path: PathBuf, size: Option<u64> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Disk,
    Image,
}

impl TargetKind {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Disk => "disk",
            Self::Image => "image",
        }
    }
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TargetInfo {
    pub kind: TargetKind,
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
    pub reason: IgnoredReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IgnoredReason {
    /// Fields the GPT repair rewrites on a drive larger than the image.
    GptRepair,
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
