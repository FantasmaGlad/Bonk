use std::io;
use std::path::Path;
use std::time::Instant;

use bonk_core::format::NewFile;
use bonk_core::format::fat32::Volume;
use bonk_core::format::fat32_tree::{self, Layout, MAX_FILE_BYTES};
use bonk_core::format::table::{self, PartitionPlan, TableKind};
use bonk_core::format::{Filesystem, FormatError, Ids};
use bonk_core::pe::{self, Machine, Signer};
use bonk_core::udf::{self, Content, ReadAt, Tree, Udf};
use bonk_core::wim::{self, Segment};
use bonk_core::write::{Control, Phase, Progress};
use bonk_core::{BlockDevice, DEFAULT_BLOCK_SIZE};
use schemars::JsonSchema;
use serde::Serialize;

use super::Target;
use super::advance::Advance;
use super::source::ImageFile;
use super::target::{Chosen, OpenRequest, reread_table};
use super::target::{OpenedForWriting, open_for_writing};
use super::translate::{guarded, support_lost};
use super::types::{Deps, JobError, TargetInfo, fail};
use super::write::WriteJob;
use crate::errors::Code;

mod copy;
mod image;
#[cfg(test)]
mod tests;

use copy::*;
pub(super) use image::{WindowsImage, detect};

/// Default loader name UEFI imposes for each machine, in the order the plan lists them.
const LOADERS: [(&str, Machine); 3] = [
    ("bootx64.efi", Machine::X86_64),
    ("bootaa64.efi", Machine::Arm64),
    ("bootia32.efi", Machine::X86),
];
const LABEL: &str = "BONK-WIN";
/// The loader fits well within this; the cap guards against an image built to deceive.
const LOADER_LIMIT: u64 = 64 << 20;
const WIM_HEADER: usize = 208;
const MIN_SIMULATED: u64 = 128 << 20;
/// Margin under the FAT32 limit: a part stays a valid file, rounding included.
const PART_LIMIT: u64 = MAX_FILE_BYTES - (1 << 20);

/// Firmware the drive boots from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Firmware {
    Uefi,
    /// Legacy BIOS, through boot code written by Bonk and the image's `bootmgr`.
    Bios,
}

impl Firmware {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Uefi => "UEFI",
            Self::Bios => "BIOS",
        }
    }
}

/// What the drive will receive, described in the plan and in the result.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Extraction {
    pub table: TableKind,
    /// UEFI always; BIOS too with an MBR table, 512-byte sectors and a `bootmgr` for x86.
    pub firmware: Vec<Firmware>,
    pub files: usize,
    pub bytes: u64,
    /// `.swm` parts; 0: the WIM fits as is.
    pub wim_parts: usize,
    /// Processors the drive boots on: one per loader that matches its own name.
    pub architectures: Vec<Machine>,
    /// Signer of the first loader of `architectures`.
    pub signer: Signer,
    /// What this certificate implies, in one sentence.
    pub secure_boot: String,
}

/// Result of the copy.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExtractReport {
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub bytes_per_second: f64,
}

/// Result of the read-back: all files, each by its hash.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExtractVerify {
    pub bytes: u64,
    pub files: usize,
    pub cache_dropped: bool,
}

/// Result of a written Windows drive.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExtractDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub extraction: Extraction,
    pub sha256_checked: bool,
    pub unmounted: Vec<String>,
    pub write: ExtractReport,
    pub verify: ExtractVerify,
    /// Always null here: present for readers of a write result.
    pub gpt_repair: Option<()>,
}

fn format_failure(code: Code, error: FormatError) -> JobError {
    if let FormatError::Device(device) = &error
        && let Some(lost) = support_lost(device)
    {
        return lost;
    }
    match error {
        FormatError::Source(err) if err.kind() == io::ErrorKind::Interrupted => fail(
            Code::Cancelled,
            "cancelled during the copy: the drive must be redone",
        ),
        FormatError::Source(err) => fail(Code::SourceUnreadable, err),
        FormatError::TooSmall(what) | FormatError::TooLarge(what) => {
            fail(Code::SourceTooLarge, what)
        }
        other => fail(code, other),
    }
}

/// An MBR beyond 2 TiB is a format error, not an image too large.
fn partition_plan(size: u64, block: u32, kind: TableKind) -> Result<PartitionPlan, JobError> {
    table::plan(size, block, kind).map_err(|error| match error {
        FormatError::TooLarge(what) => fail(Code::FormatFailed, what),
        other => format_failure(Code::WriteFailed, other),
    })
}

pub(super) fn run(
    prepared: &WindowsImage,
    file: &ImageFile,
    chosen: Chosen,
    job: &WriteJob,
    sha256_checked: bool,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<ExtractDone, JobError> {
    // Simulation file to create: large enough for the volume, sparse, never less than FAT32 needs.
    let size_hint = prepared
        .describe(job.table, DEFAULT_BLOCK_SIZE)
        .bytes
        .saturating_mul(2)
        .max(MIN_SIMULATED);
    let request =
        OpenRequest::read_write(job.options, job.elevation, job.trace_io, Some(size_hint));
    let OpenedForWriting {
        mut device,
        info,
        unmounted,
        ids,
        ..
    } = open_for_writing(chosen, &job.target, &request, deps, "extracting")?;
    let (layout, volume, extraction) = write_volume(&mut *device, prepared, job.table, &ids)?;
    let write_failure = |e| format_failure(Code::WriteFailed, e);
    let total = layout.data_bytes();
    let mut report = guarded(deps, progress);
    let started = Instant::now();
    let sums = write_files(&mut *device, &layout, prepared, file, started, &mut report)?;
    layout
        .finish(&mut *device, &volume)
        .map_err(write_failure)?;
    device.flush().map_err(|e| fail(Code::WriteFailed, e))?;
    let elapsed = started.elapsed();
    let write = ExtractReport {
        bytes: total,
        elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        bytes_per_second: total as f64 / elapsed.as_secs_f64().max(0.001),
    };
    let verify = verify_all(&mut *device, &layout, &volume, &sums, total, &mut report)?;
    drop(device);
    if matches!(job.target, Target::Disk(_)) {
        reread_table(Path::new(&info.path), job.elevation, deps);
    }
    log::info!(target: "bonk::write", files = sums.len(), bytes = total; "windows key written");
    Ok(ExtractDone {
        schema_version: super::types::SCHEMA_VERSION,
        event: "result",
        target: info,
        extraction,
        sha256_checked,
        unmounted,
        write,
        verify,
        gpt_repair: None,
    })
}
