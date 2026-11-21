use std::io::{self, Read};

use bonk_core::efi::{LOADER_LIMIT, LOADERS};
use bonk_core::format::NewFile;
use bonk_core::format::fat32::Volume;
use bonk_core::format::fat32::tree::{self, MAX_FILE_BYTES};
use bonk_core::format::table::TableKind;
use bonk_core::pe::{self, Machine, Signer};
use bonk_core::udf::{self, Content, Tree, Udf};
use bonk_core::wim::{self, Segment};

use super::{Extraction, Firmware, LABEL, PART_LIMIT, WIM_HEADER, format_failure, partition_plan};
use crate::errors::Code;
use crate::job::source::ImageFile;
use crate::job::types::{JobError, fail};

pub(super) struct Item {
    pub(super) path: Vec<String>,
    pub(super) size: u64,
    pub(super) from: From,
}

pub(super) enum From {
    Iso(Content),
    Part(Vec<Segment>, Content),
}

pub(super) struct ItemReader<'a> {
    pub(super) image: &'a ImageFile,
    pub(super) item: &'a Item,
    pub(super) position: u64,
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

pub(in crate::job) struct WindowsImage {
    pub(super) items: Vec<Item>,
    pub(super) directories: Vec<Vec<String>>,
    pub(super) signer: Signer,
    pub(super) architectures: Vec<Machine>,
    pub(super) wim_parts: usize,
    pub(super) bios_loader: bool,
}

pub(super) fn read_whole(image: &ImageFile, content: &Content, size: u64) -> io::Result<Vec<u8>> {
    let mut bytes = vec![0; usize::try_from(size).map_err(|_| io::ErrorKind::InvalidData)?];
    udf::read_content(image, content, 0, &mut bytes)?;
    Ok(bytes)
}

pub(super) fn wim_error(error: impl std::fmt::Display) -> JobError {
    fail(Code::SourceFormatUnsupported, error)
}

/// Installer image first: an installer also carries the Windows PE system.
const WIMS: [&str; 3] = ["install.wim", "install.esd", "boot.wim"];

pub(in crate::job) fn detect(image: &ImageFile) -> Result<Option<WindowsImage>, JobError> {
    let unreadable = |e: &dyn std::fmt::Display| fail(Code::SourceUnreadable, e);
    let Ok(udf) = Udf::open(image) else {
        return Ok(None);
    };
    let Ok(tree) = udf.tree() else {
        return Ok(None);
    };
    let Some(wim) = WIMS.iter().find_map(|name| tree.find(&["sources", name])) else {
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

pub(super) fn plan_items(
    image: &ImageFile,
    tree: &Tree,
    wim_file: &udf::File,
) -> Result<(Vec<Item>, usize), JobError> {
    let mut items: Vec<Item> = tree
        .files
        .iter()
        .filter(|file| file.path != wim_file.path)
        .map(|file| Item {
            path: file.path.clone(),
            size: file.size,
            from: From::Iso(file.content.clone()),
        })
        .collect();
    if wim_file.size <= MAX_FILE_BYTES {
        items.push(Item {
            path: wim_file.path.clone(),
            size: wim_file.size,
            from: From::Iso(wim_file.content.clone()),
        });
        return Ok((items, 0));
    }
    let parts = wim_parts(image, wim_file)?;
    let count = parts.len();
    items.extend(parts);
    Ok((items, count))
}

/// `install.swm`, `install2.swm`, … in the folder of the WIM they split.
fn wim_parts(image: &ImageFile, wim_file: &udf::File) -> Result<Vec<Item>, JobError> {
    let read = |offset, len: u64| -> Result<Vec<u8>, JobError> {
        let mut bytes = vec![0; len as usize];
        udf::read_content(image, &wim_file.content, offset, &mut bytes)
            .map_err(|e| fail(Code::SourceUnreadable, e))?;
        Ok(bytes)
    };
    let header = read(0, WIM_HEADER as u64)?;
    let info = wim::read_header(&header, wim_file.size).map_err(wim_error)?;
    let lookup = read(info.lookup_offset, info.lookup_len)?;
    let parts = wim::split(&header, &lookup, wim_file.size, PART_LIMIT).map_err(wim_error)?;
    let (_, parent) = wim_file
        .path
        .split_last()
        .ok_or_else(|| wim_error("empty path"))?;
    Ok(parts
        .into_iter()
        .enumerate()
        .map(|(index, segments)| {
            let file = if index == 0 {
                "install.swm".to_owned()
            } else {
                format!("install{}.swm", index + 1)
            };
            let mut path = parent.to_vec();
            path.push(file);
            Item {
                path,
                size: segments.iter().map(Segment::len).sum(),
                from: From::Part(segments, wim_file.content.clone()),
            }
        })
        .collect())
}

impl WindowsImage {
    pub(super) fn new_files(&self) -> Vec<NewFile> {
        self.items
            .iter()
            .map(|item| NewFile {
                path: item.path.clone(),
                size: item.size,
            })
            .collect()
    }

    pub(super) fn firmware(&self, table: TableKind, block: u32) -> Vec<Firmware> {
        let bios = self.bios_loader && table == TableKind::Mbr && block == bonk_core::boot::SECTOR;
        let mut firmware = vec![Firmware::Uefi];
        if bios {
            firmware.push(Firmware::Bios);
        }
        firmware
    }

    pub(in crate::job) fn describe(&self, table: TableKind, block: u32) -> Extraction {
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

    pub(in crate::job) fn check_fits(
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
        tree::layout(&volume, block, &self.new_files(), &self.directories)
            .map(drop)
            .map_err(|e| format_failure(Code::WriteFailed, e))
    }
}
