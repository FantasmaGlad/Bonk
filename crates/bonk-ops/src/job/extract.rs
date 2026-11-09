use std::io::{self, Read};
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
use super::format::fresh_seed;
use super::source::ImageFile;
use super::target::{Chosen, OpenRequest, open_target, reread_table};
use super::translate::{guarded, support_lost};
use super::types::{Deps, JobError, TargetInfo, fail};
use super::write::WriteJob;
use crate::errors::Code;

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

impl ReadAt for ImageFile {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.read_exact_positioned(offset, buf)
    }
}

/// As is from the ISO, or a WIM part.
struct Item {
    path: Vec<String>,
    size: u64,
    from: From,
}

enum From {
    Iso(Content),
    Part(Vec<Segment>, Content),
}

struct ItemReader<'a> {
    image: &'a ImageFile,
    item: &'a Item,
    position: u64,
}

impl Read for ItemReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.item.size - self.position;
        let want = left.min(buf.len() as u64) as usize;
        if want == 0 {
            return Ok(0);
        }
        let take = match &self.item.from {
            From::Iso(content) => {
                udf::read_content(self.image, content, self.position, &mut buf[..want])?;
                want
            }
            From::Part(segments, wim_content) => {
                let mut skip = self.position;
                let mut taken = 0;
                for segment in segments {
                    if skip >= segment.len() {
                        skip -= segment.len();
                        continue;
                    }
                    let take = (segment.len() - skip).min(want as u64) as usize;
                    match segment {
                        Segment::Bytes(bytes) => {
                            let start = skip as usize;
                            buf[..take].copy_from_slice(&bytes[start..start + take]);
                        }
                        Segment::Source { offset, .. } => udf::read_content(
                            self.image,
                            wim_content,
                            offset + skip,
                            &mut buf[..take],
                        )?,
                    }
                    taken = take;
                    break;
                }
                taken
            }
        };
        self.position += take as u64;
        Ok(take)
    }
}

/// Where every byte of the prepared ISO goes.
pub(super) struct WindowsImage {
    items: Vec<Item>,
    directories: Vec<Vec<String>>,
    signer: Signer,
    architectures: Vec<Machine>,
    wim_parts: usize,
    /// `bootmgr` at the root, for x86: the loader the BIOS code starts.
    bios_loader: bool,
}

fn read_whole(image: &ImageFile, content: &Content, size: u64) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(size).map_err(|_| io::ErrorKind::InvalidData)?];
    udf::read_content(image, content, 0, &mut bytes)?;
    Ok(bytes)
}

fn wim_error(error: impl std::fmt::Display) -> JobError {
    fail(Code::SourceFormatUnsupported, error)
}

/// `Ok(None)`: not a Windows installation ISO Bonk can write.
pub(super) fn detect(image: &ImageFile) -> Result<Option<WindowsImage>, JobError> {
    let unreadable = |e: &dyn std::fmt::Display| fail(Code::SourceUnreadable, e);
    let Ok(udf) = Udf::open(image) else {
        return Ok(None);
    };
    let Ok(tree) = udf.tree() else {
        return Ok(None);
    };
    let wim = tree
        .find(&["sources", "install.wim"])
        .or_else(|| tree.find(&["sources", "install.esd"]));
    let Some(wim) = wim else {
        return Ok(None);
    };
    let mut architectures = Vec::new();
    let mut signer = None;
    for (name, expected) in LOADERS {
        let Some(loader) = tree.find(&["efi", "boot", name]) else {
            continue;
        };
        if loader.size > LOADER_LIMIT {
            continue;
        }
        let bytes = read_whole(image, &loader.content, loader.size).map_err(|e| unreadable(&e))?;
        // The firmware picks the file by name: a loader for another machine would not boot there.
        if pe::machine(&bytes) != Some(expected) {
            continue;
        }
        architectures.push(expected);
        signer.get_or_insert_with(|| pe::signer(&bytes).unwrap_or(Signer::Unsigned));
    }
    let Some(signer) = signer else {
        return Ok(None);
    };
    let bios_loader = tree.find(&["bootmgr"]).is_some()
        && architectures
            .iter()
            .any(|machine| matches!(machine, Machine::X86_64 | Machine::X86));
    let (items, wim_parts) = plan_items(image, &tree, wim)?;
    Ok(Some(WindowsImage {
        items,
        directories: tree.directories.clone(),
        signer,
        architectures,
        wim_parts,
        bios_loader,
    }))
}

/// A WIM of 4 GiB or more becomes parts next to it.
fn plan_items(
    image: &ImageFile,
    tree: &Tree,
    wim_file: &udf::File,
) -> Result<(Vec<Item>, usize), JobError> {
    let mut items = Vec::with_capacity(tree.files.len());
    for file in &tree.files {
        if file.path != wim_file.path {
            items.push(Item {
                path: file.path.clone(),
                size: file.size,
                from: From::Iso(file.content.clone()),
            });
        }
    }
    if wim_file.size <= MAX_FILE_BYTES {
        items.push(Item {
            path: wim_file.path.clone(),
            size: wim_file.size,
            from: From::Iso(wim_file.content.clone()),
        });
        return Ok((items, 0));
    }
    let read = |offset, len: u64| -> Result<Vec<u8>, JobError> {
        let mut bytes = vec![0; len as usize];
        udf::read_content(image, &wim_file.content, offset, &mut bytes)
            .map_err(|e| fail(Code::SourceUnreadable, e))?;
        Ok(bytes)
    };
    let header = read(0, WIM_HEADER as u64)?;
    let info = wim::read_header(&header).map_err(wim_error)?;
    let lookup = read(info.lookup_offset, info.lookup_len)?;
    let parts = wim::split(&header, &lookup, wim_file.size, PART_LIMIT).map_err(wim_error)?;
    let count = parts.len();
    let (_, parent) = wim_file
        .path
        .split_last()
        .ok_or_else(|| wim_error("empty path"))?;
    for (index, segments) in parts.into_iter().enumerate() {
        let file = if index == 0 {
            "install.swm".to_owned()
        } else {
            format!("install{}.swm", index + 1)
        };
        let mut path = parent.to_vec();
        path.push(file);
        items.push(Item {
            path,
            size: segments.iter().map(Segment::len).sum(),
            from: From::Part(segments, wim_file.content.clone()),
        });
    }
    Ok((items, count))
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

impl WindowsImage {
    fn new_files(&self) -> Vec<NewFile> {
        self.items
            .iter()
            .map(|item| NewFile {
                path: item.path.clone(),
                size: item.size,
            })
            .collect()
    }

    fn firmware(&self, table: TableKind, block: u32) -> Vec<Firmware> {
        let bios = self.bios_loader && table == TableKind::Mbr && block == bonk_core::boot::SECTOR;
        let mut firmware = vec![Firmware::Uefi];
        if bios {
            firmware.push(Firmware::Bios);
        }
        firmware
    }

    pub(super) fn describe(&self, table: TableKind, block: u32) -> Extraction {
        Extraction {
            table,
            firmware: self.firmware(table, block),
            files: self.items.len(),
            bytes: self.items.iter().map(|item| item.size).sum(),
            wim_parts: self.wim_parts,
            architectures: self.architectures.clone(),
            secure_boot: self.signer.meaning(),
            signer: self.signer.clone(),
        }
    }

    /// Without touching the target.
    pub(super) fn check_fits(
        &self,
        size: Option<u64>,
        block: u32,
        table_kind: TableKind,
    ) -> Result<(), JobError> {
        let Some(size) = size else { return Ok(()) };
        let partition = partition_plan(size, block, table_kind)?;
        let volume = Volume {
            first_lba: partition.first_lba,
            sectors: partition.sectors(),
            label: Some(LABEL),
            volume_id: 0,
            bios_boot: false,
        };
        fat32_tree::layout(&volume, block, &self.new_files(), &self.directories)
            .map(drop)
            .map_err(|e| format_failure(Code::WriteFailed, e))
    }
}

/// Shared by the copy and the read-back.
struct Advance<'a, 'b> {
    phase: Phase,
    done: u64,
    total: u64,
    started: Instant,
    report: &'a mut (dyn FnMut(&Progress) -> Control + 'b),
}

impl Advance<'_, '_> {
    fn add(&mut self, bytes: u64) -> bool {
        self.done += bytes;
        let elapsed = self.started.elapsed();
        let event = Progress {
            phase: self.phase,
            bytes_done: self.done,
            bytes_total: Some(self.total),
            bytes_per_second: self.done as f64 / elapsed.as_secs_f64().max(0.001),
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        };
        (self.report)(&event) != Control::Cancel
    }
}

/// Table, volume, files, then each file read back with the cache dropped.
pub(super) fn run(
    prepared: &WindowsImage,
    file: &ImageFile,
    chosen: Chosen,
    job: &WriteJob,
    sha256_checked: bool,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<ExtractDone, JobError> {
    let seed_name = chosen.info.path.clone();
    let (mut device, info, unmounted) = open_target(
        chosen,
        &job.target,
        &OpenRequest {
            options: job.options,
            access: bonk_hal::Access::ReadWrite,
            elevation: job.elevation,
            // Simulation file to create: large enough for the volume, sparse, never less than FAT32 needs.
            size_hint: Some(
                prepared
                    .describe(job.table, DEFAULT_BLOCK_SIZE)
                    .bytes
                    .saturating_mul(2)
                    .max(MIN_SIMULATED),
            ),
            trace_io: job.trace_io,
        },
        deps,
    )?;
    let block = device.block_size();
    let files = prepared.new_files();
    let ids = Ids::from_seed(&fresh_seed(&seed_name));
    let partition = partition_plan(device.size(), block, job.table)?;
    let extraction = prepared.describe(job.table, block);
    let bios_boot = extraction.firmware.contains(&Firmware::Bios);
    let volume = Volume {
        first_lba: partition.first_lba,
        sectors: partition.sectors(),
        label: Some(LABEL),
        volume_id: ids.volume_id,
        bios_boot,
    };
    // Everything is computed before the first write: a refusal costs the key nothing.
    let layout = fat32_tree::layout(&volume, block, &files, &prepared.directories)
        .map_err(|e| format_failure(Code::WriteFailed, e))?;
    let write_failure = |e| format_failure(Code::WriteFailed, e);
    table::write_table(
        &mut *device,
        job.table,
        Filesystem::Fat32,
        Some(LABEL),
        &ids,
        bios_boot,
    )
    .map_err(write_failure)?;
    layout
        .write_structures(&mut *device)
        .map_err(write_failure)?;

    let total = layout.data_bytes();
    let mut report = guarded(deps, progress);
    let started = Instant::now();
    let mut sums = Vec::with_capacity(prepared.items.len());
    {
        let mut advance = Advance {
            phase: Phase::Write,
            done: 0,
            total,
            started,
            report: &mut report,
        };
        for (index, item) in prepared.items.iter().enumerate() {
            let mut reader = ItemReader {
                image: file,
                item,
                position: 0,
            };
            let sum = layout
                .write_file(&mut *device, index, &mut reader, &mut |n| advance.add(n))
                .map_err(write_failure)?;
            sums.push(sum);
        }
    }
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

fn verify_all(
    device: &mut dyn BlockDevice,
    layout: &Layout,
    volume: &Volume<'_>,
    sums: &[String],
    total: u64,
    report: &mut dyn FnMut(&Progress) -> Control,
) -> Result<ExtractVerify, JobError> {
    let cache_dropped = device.drop_cache().is_ok();
    if !cache_dropped {
        log::warn!(target: "bonk::write", "cache not dropped: the verification may have read cached data");
    }
    let verify_failure = |e| format_failure(Code::VerifyMismatch, e);
    layout
        .verify_structures(device, volume)
        .map_err(verify_failure)?;
    if volume.bios_boot {
        table::verify_bios_boot(device).map_err(verify_failure)?;
    }
    let mut advance = Advance {
        phase: Phase::Verify,
        done: 0,
        total,
        started: Instant::now(),
        report,
    };
    for (index, sum) in sums.iter().enumerate() {
        layout
            .verify_file(device, index, sum, &mut |n| advance.add(n))
            .map_err(verify_failure)?;
    }
    Ok(ExtractVerify {
        bytes: total,
        files: sums.len(),
        cache_dropped,
    })
}

#[cfg(test)]
mod tests {
    use bonk_core::BlockError;

    use super::*;

    /// The BIOS code assumes 512-byte sectors: on a 4 KiB-sector disk the plan does not announce it.
    #[test]
    fn le_bios_n_est_annonce_qu_avec_des_secteurs_de_512_octets() {
        let image = WindowsImage {
            items: Vec::new(),
            directories: Vec::new(),
            signer: Signer::Unsigned,
            architectures: vec![Machine::X86_64],
            wim_parts: 0,
            bios_loader: true,
        };
        assert_eq!(
            image.describe(TableKind::Mbr, 512).firmware,
            [Firmware::Uefi, Firmware::Bios]
        );
        assert_eq!(
            image.describe(TableKind::Mbr, 4096).firmware,
            [Firmware::Uefi]
        );
        assert_eq!(
            image.describe(TableKind::Gpt, 512).firmware,
            [Firmware::Uefi]
        );
    }

    #[test]
    fn un_mbr_au_dela_de_2_tio_est_refuse_par_le_code_du_formatage() {
        let error = partition_plan(4 << 40, 512, TableKind::Mbr).err();
        assert_eq!(error.map(|e| e.code), Some(Code::FormatFailed));
        assert!(partition_plan(4 << 40, 512, TableKind::Gpt).is_ok());
    }

    /// Same message as an image write: the kernel answers "no space" on an unplugged disk.
    #[test]
    fn un_support_disparu_pendant_la_copie_est_dit() {
        let full = io::Error::from(io::ErrorKind::StorageFull);
        let error = format_failure(Code::WriteFailed, FormatError::Device(BlockError::Io(full)));
        assert_eq!(error.code, Code::WriteFailed);
        assert!(error.message.contains("unplugged"), "{}", error.message);
    }
}
