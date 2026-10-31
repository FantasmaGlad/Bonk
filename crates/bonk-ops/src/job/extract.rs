//! Clé d'installation Windows : l'ISO n'est pas écrite telle quelle, elle n'amorce pas depuis un disque. Ses
//! fichiers sont copiés sur une partition FAT32 écrite ici ; un `install.wim` trop grand pour FAT32 est
//! découpé en `.swm`, sans recompression. Le démarrage se fait en UEFI, par le chargeur de l'ISO.

use std::io::{self, Read};
use std::path::Path;
use std::time::Instant;

use bonk_core::format::fat32::Volume;
use bonk_core::format::fat32_tree::{self, Layout, MAX_FILE_BYTES, NewFile};
use bonk_core::format::table::{self, TableKind};
use bonk_core::format::{Filesystem, FormatError, Ids};
use bonk_core::pe::{self, Signer};
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
use super::translate::guarded;
use super::types::{Deps, JobError, TargetInfo, fail};
use super::write::WriteJob;
use crate::errors::Code;

const LOADER: [&str; 3] = ["efi", "boot", "bootx64.efi"];
const LABEL: &str = "BONK-WIN";
/// Le chargeur tient largement là-dedans ; le plafond protège d'une image construite pour tromper.
const LOADER_LIMIT: u64 = 64 << 20;
const WIM_HEADER: usize = 208;
const MIN_SIMULATED: u64 = 128 << 20;
/// Marge sous la limite de FAT32 : un morceau reste un fichier valide, arrondi compris.
const PART_LIMIT: u64 = MAX_FILE_BYTES - (1 << 20);

/// Ce que la clé recevra, décrit dans le plan et dans le résultat.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Extraction {
    pub table: TableKind,
    pub files: usize,
    pub bytes: u64,
    /// Morceaux `.swm` ; 0 : le WIM tient tel quel.
    pub wim_parts: usize,
    pub signer: Signer,
    /// Ce que ce certificat implique, en une phrase.
    pub secure_boot: String,
}

/// Résultat de la copie.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExtractReport {
    pub bytes: u64,
    pub elapsed_ms: u64,
    pub bytes_per_second: f64,
}

/// Résultat de la relecture : tous les fichiers, chacun par son empreinte.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ExtractVerify {
    pub bytes: u64,
    pub files: usize,
    pub cache_dropped: bool,
}

/// Résultat d'une clé Windows écrite.
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
    /// Toujours nul ici : présent pour les lecteurs du résultat d'une écriture.
    pub gpt_repair: Option<()>,
}

impl ReadAt for ImageFile {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.read_exact_positioned(offset, buf)
    }
}

/// Un fichier de la clé : tel quel depuis l'ISO, ou un morceau de WIM.
struct Item {
    path: Vec<String>,
    size: u64,
    from: From,
}

enum From {
    Iso(Content),
    Part(Vec<Segment>, Content),
}

/// Lit un élément d'un bout à l'autre.
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

/// L'ISO préparée : où va chaque octet.
pub(super) struct WindowsImage {
    items: Vec<Item>,
    directories: Vec<Vec<String>>,
    signer: Signer,
    wim_parts: usize,
}

fn read_whole(image: &ImageFile, content: &Content, size: u64) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(size).map_err(|_| io::ErrorKind::InvalidData)?];
    udf::read_content(image, content, 0, &mut bytes)?;
    Ok(bytes)
}

fn wim_error(error: impl std::fmt::Display) -> JobError {
    fail(Code::SourceFormatUnsupported, error)
}

/// `Ok(None)` : ce n'est pas une ISO d'installation de Windows que Bonk sait écrire.
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
    let (Some(loader), Some(wim)) = (tree.find(&LOADER), wim) else {
        return Ok(None);
    };
    if loader.size > LOADER_LIMIT {
        return Ok(None);
    }
    let bytes = read_whole(image, &loader.content, loader.size).map_err(|e| unreadable(&e))?;
    if !pe::is_x86_64(&bytes) {
        return Ok(None);
    }
    let signer = pe::signer(&bytes).unwrap_or(Signer::Unsigned);
    let (items, wim_parts) = plan_items(image, &tree, wim)?;
    Ok(Some(WindowsImage {
        items,
        directories: tree.directories.clone(),
        signer,
        wim_parts,
    }))
}

/// Tous les fichiers de l'ISO ; un WIM de 4 Gio ou plus devient des morceaux à côté de lui.
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
        .ok_or_else(|| wim_error("chemin vide"))?;
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
    match error {
        FormatError::Source(err) if err.kind() == io::ErrorKind::Interrupted => fail(
            Code::Cancelled,
            "annulée pendant la copie : la clé est à refaire",
        ),
        FormatError::Source(err) => fail(Code::SourceUnreadable, err),
        FormatError::TooSmall(what) | FormatError::TooLarge(what) => {
            fail(Code::SourceTooLarge, what)
        }
        other => fail(code, other),
    }
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

    /// Ce que le plan et le résultat disent de la clé.
    pub(super) fn describe(&self, table: TableKind) -> Extraction {
        Extraction {
            table,
            files: self.items.len(),
            bytes: self.items.iter().map(|item| item.size).sum(),
            wim_parts: self.wim_parts,
            secure_boot: self.signer.meaning(),
            signer: self.signer.clone(),
        }
    }

    /// Contrôle que tout tient sur la cible, sans la toucher.
    pub(super) fn check_fits(
        &self,
        size: Option<u64>,
        table_kind: TableKind,
    ) -> Result<(), JobError> {
        let Some(size) = size else { return Ok(()) };
        let partition = table::plan(size, DEFAULT_BLOCK_SIZE, table_kind)
            .map_err(|e| format_failure(Code::WriteFailed, e))?;
        let volume = Volume {
            first_lba: partition.first_lba,
            sectors: partition.sectors(),
            label: Some(LABEL),
            volume_id: 0,
        };
        fat32_tree::layout(
            &volume,
            DEFAULT_BLOCK_SIZE,
            &self.new_files(),
            &self.directories,
        )
        .map(drop)
        .map_err(|e| format_failure(Code::WriteFailed, e))
    }
}

/// Avancement commun à la copie et à la relecture.
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

/// Écrit la clé : table, volume, fichiers, puis relecture de chaque fichier, cache vidé.
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
            // Fichier de simulation à créer : assez grand pour le volume, creux, jamais moins de ce que
            // FAT32 exige.
            size_hint: Some(
                prepared
                    .describe(job.table)
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
    let partition = table::plan(device.size(), block, job.table)
        .map_err(|e| format_failure(Code::WriteFailed, e))?;
    let volume = Volume {
        first_lba: partition.first_lba,
        sectors: partition.sectors(),
        label: Some(LABEL),
        volume_id: ids.volume_id,
    };
    // Tout est calculé avant le premier octet écrit : un refus ne coûte rien à la clé.
    let layout = fat32_tree::layout(&volume, block, &files, &prepared.directories)
        .map_err(|e| format_failure(Code::WriteFailed, e))?;
    let write_failure = |e| format_failure(Code::WriteFailed, e);
    table::write_table(
        &mut *device,
        job.table,
        Filesystem::Fat32,
        Some(LABEL),
        &ids,
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
        extraction: prepared.describe(job.table),
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
