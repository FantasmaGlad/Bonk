use std::time::Instant;

use bonk_core::BlockDevice;
use bonk_core::format::fat32::Volume;
use bonk_core::format::fat32::tree::{self, Layout};
use bonk_core::format::table::{self, TableKind};
use bonk_core::format::{Filesystem, Ids};
use bonk_core::write::{Control, Phase, Progress};

use super::image::{ItemReader, WindowsImage};
use super::{ExtractVerify, Extraction, Firmware, LABEL, format_failure, partition_plan};
use crate::errors::Code;
use crate::job::advance::Advance;
use crate::job::source::ImageFile;
use crate::job::types::JobError;

/// Everything is computed before the first write: a refusal costs the key nothing. Then the table and
/// the FAT32 structures; the files follow.
pub(super) fn write_volume<'a>(
    device: &mut dyn BlockDevice,
    prepared: &WindowsImage,
    kind: TableKind,
    ids: &Ids,
) -> Result<(Layout, Volume<'a>, Extraction), JobError> {
    let block = device.block_size();
    let files = prepared.new_files();
    let partition = partition_plan(device.size(), block, kind)?;
    let extraction = prepared.describe(kind, block);
    let bios_boot = extraction.firmware.contains(&Firmware::Bios);
    let volume = Volume {
        first_lba: partition.first_lba,
        sectors: partition.sectors(),
        label: Some(LABEL),
        volume_id: ids.volume_id,
        bios_boot,
    };
    let write_failure = |e| format_failure(Code::WriteFailed, e);
    let layout =
        tree::layout(&volume, block, &files, &prepared.directories).map_err(write_failure)?;
    table::write_table(device, kind, Filesystem::Fat32, Some(LABEL), ids, bios_boot)
        .map_err(write_failure)?;
    layout.write_structures(device).map_err(write_failure)?;
    Ok((layout, volume, extraction))
}

/// Each file's SHA-256, in order, for the read-back.
pub(super) fn write_files(
    device: &mut dyn BlockDevice,
    layout: &Layout,
    prepared: &WindowsImage,
    file: &ImageFile,
    started: Instant,
    report: &mut dyn FnMut(&Progress) -> Control,
) -> Result<Vec<String>, JobError> {
    let mut sums = Vec::with_capacity(prepared.items.len());
    let mut advance = Advance {
        phase: Phase::Write,
        done: 0,
        total: layout.data_bytes(),
        started,
        report,
    };
    for (index, item) in prepared.items.iter().enumerate() {
        let mut reader = ItemReader {
            image: file,
            item,
            position: 0,
        };
        let sum = layout
            .write_file(device, index, &mut reader, &mut |n| advance.add(n))
            .map_err(|e| format_failure(Code::WriteFailed, e))?;
        sums.push(sum);
    }
    Ok(sums)
}

pub(super) fn verify_all(
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
