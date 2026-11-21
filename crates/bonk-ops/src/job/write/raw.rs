use std::path::Path;

use bonk_core::BlockDevice;
use bonk_core::codec::{Format, Source};
use bonk_core::partition::inspect;
use bonk_core::repair::{RepairReport, needs_repair, repair_gpt};
use bonk_core::write::{self as engine, Control, DEFAULT_CHUNK, Phase, Progress};

use crate::errors::Code;
use crate::job::target::{Chosen, OpenRequest, open_target, reread_table};
use crate::job::translate::{guarded, map_verify_error, map_write_error};
use crate::job::types::{Deps, JobError, SCHEMA_VERSION, Target, WriteDone, WriteOutcome, fail};
use crate::job::verify::{drop_cache_before_verify, warn_if_cache_kept};

use super::*;

pub(in crate::job) fn announce_authorization(
    total: Option<u64>,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(), JobError> {
    let waiting = Progress {
        phase: Phase::Authorize,
        bytes_done: 0,
        bytes_total: total,
        bytes_per_second: 0.0,
        elapsed_ms: 0,
    };
    if guarded(deps, progress)(&waiting) == Control::Cancel {
        return Err(fail(Code::Cancelled, "cancelled before opening the disk"));
    }
    Ok(())
}

pub(in crate::job) fn write_raw(
    job: &WriteJob,
    chosen: Chosen,
    mut source: Source,
    checked: Checked,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteOutcome, JobError> {
    let Checked {
        source_info,
        image,
        sha256_checked,
        hybrid_mbr,
        efi_loaders,
    } = checked;
    log::info!(
        target: "bonk::write",
        target_kind = chosen.info.kind.name(), target_path = chosen.info.path.as_str(),
        source = source_info.path.as_str(), format = source_info.format.name();
        "write starting"
    );
    if chosen.disk.is_some() {
        announce_authorization(source_info.size_bytes, deps, progress)?;
    }
    let request = OpenRequest::read_write(
        job.options,
        job.elevation,
        job.trace_io,
        source_info.size_bytes,
    );
    let (mut device, info, unmounted) = open_target(chosen, &job.target, &request, deps)?;
    let mut report_progress = guarded(deps, progress);
    let write = engine::write_image(
        &mut source.reader,
        &mut *device,
        source_info.size_bytes,
        DEFAULT_CHUNK,
        &mut report_progress,
    )
    .map_err(|e| map_write_error(e, source_info.format))?;
    log::info!(target: "bonk::write", bytes = write.bytes, millis = write.elapsed_ms; "write finished");
    check_complete(&image, write.bytes, job.mode)?;
    let verify = if job.verify {
        Some(verify_written(
            &mut *device,
            &write,
            source_info.format,
            &mut report_progress,
        )?)
    } else {
        None
    };
    if let Some(mbr) = &hybrid_mbr {
        write_hybrid_mbr(&mut *device, mbr)?;
    }
    let gpt_repair = if job.repair_gpt {
        repair_if_needed(&mut *device)?
    } else {
        None
    };
    drop(device);
    if matches!(job.target, Target::Disk(_)) {
        reread_table(Path::new(&info.path), job.elevation, deps);
    }
    Ok(WriteOutcome::Done(Box::new(WriteDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        target: info,
        source: source_info,
        image,
        sha256_checked,
        unmounted,
        write,
        verify,
        gpt_repair,
        hybrid_mbr: hybrid_mbr.is_some(),
        efi_loaders,
    })))
}

/// Once the ISO is written and read back: its first sector was empty, it becomes the MBR. A drive with
/// 4096-byte sectors gets the whole first block, the rest of it unchanged.
fn write_hybrid_mbr(device: &mut dyn BlockDevice, mbr: &[u8]) -> Result<(), JobError> {
    let failed =
        |e: bonk_core::block::BlockError| fail(Code::WriteFailed, format!("hybrid MBR: {e}"));
    let mut block = vec![0u8; device.block_size() as usize];
    device.read_at(0, &mut block).map_err(failed)?;
    block[..mbr.len()].copy_from_slice(mbr);
    device.write_at(0, &block).map_err(failed)?;
    device.flush().map_err(failed)?;
    drop_cache_before_verify(device);
    let mut back = vec![0u8; mbr.len()];
    device.read_at(0, &mut back).map_err(failed)?;
    if back != mbr {
        return Err(fail(
            Code::VerifyMismatch,
            "the hybrid MBR did not read back as written",
        ));
    }
    log::info!(target: "bonk::write", "hybrid mbr written and read back");
    Ok(())
}

pub(in crate::job) fn verify_written(
    device: &mut dyn BlockDevice,
    write: &engine::WriteReport,
    format: Format,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<engine::VerifyReport, JobError> {
    let report = engine::verify(device, write.bytes, &write.sha256, DEFAULT_CHUNK, progress)
        .map_err(|e| map_verify_error(e, format))?;
    log::info!(target: "bonk::write", bytes = report.bytes; "verification passed");
    warn_if_cache_kept(&report);
    Ok(report)
}

/// The drive is larger than the image: the backup GPT is no longer at the end.
pub(in crate::job) fn repair_if_needed(
    device: &mut dyn BlockDevice,
) -> Result<Option<RepairReport>, JobError> {
    let layout = inspect(device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    if !needs_repair(&layout) {
        return Ok(None);
    }
    let report = repair_gpt(device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    // Read back after flushing: a drive that did not keep the tables would let a key without a backup GPT go
    // out with a success.
    device.flush().map_err(|e| fail(Code::GptRepairFailed, e))?;
    drop_cache_before_verify(device);
    let after = inspect(device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    if needs_repair(&after) {
        return Err(fail(
            Code::GptRepairFailed,
            "the repaired GPT did not read back: the drive may not keep what is written at its end",
        ));
    }
    log::info!(target: "bonk::write", actions = report.actions.join(",").as_str(); "gpt repaired and read back");
    Ok(Some(report))
}
