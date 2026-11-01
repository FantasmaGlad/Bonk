//! Comparer un support à une image, sans écrire.

use std::path::PathBuf;

use bonk_core::BlockDevice;
use bonk_core::write::{self as engine, Control, DEFAULT_CHUNK, Progress};
use bonk_hal::{Access, Elevation, FilterOptions};

use super::source::ImageFile;
use super::target::{OpenRequest, choose, open_target};
use super::translate::{guarded, map_verify_error};
use super::types::{
    Deps, IgnoredRange, JobError, SCHEMA_VERSION, Target, TraceIo, VerifyDone, fail,
};
use crate::errors::Code;

pub struct VerifyJob {
    pub target: Target,
    pub source: PathBuf,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub elevation: Elevation,
}

pub fn verify(
    job: &VerifyJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VerifyDone, JobError> {
    let file = ImageFile::open(&job.source)?;
    let (mut source, source_info) = file.decoded()?;
    let chosen = choose(&job.target, job.options, deps)?;
    let (mut device, info, _) = open_target(
        chosen,
        &job.target,
        &OpenRequest {
            options: job.options,
            access: Access::ReadOnly,
            elevation: job.elevation,
            size_hint: None,
            trace_io: job.trace_io,
        },
        deps,
    )?;
    let mut report_progress = guarded(deps, progress);
    // Une image GPT posée sur un support plus grand a vu quelques champs réécrits par la réparation du
    // GPT (MBR de protection, positions de l'en-tête) : les comparer à l'image donnerait un faux échec.
    let ignore = if source_info.size_bytes.is_none_or(|n| n < device.size()) {
        repair_ranges(&file)
    } else {
        Vec::new()
    };
    let (len, sha) = engine::hash_source_ignoring(
        &mut source.reader,
        source_info.size_bytes,
        DEFAULT_CHUNK,
        &ignore,
        &mut report_progress,
    )
    .map_err(|e| map_verify_error(e, source_info.format))?;
    if len > device.size() {
        return Err(fail(
            Code::SourceTooLarge,
            format!(
                "the image is {len} bytes, the target {} bytes",
                device.size()
            ),
        ));
    }
    let verify = engine::verify_ignoring(
        &mut *device,
        len,
        &sha,
        DEFAULT_CHUNK,
        &ignore,
        &mut report_progress,
    )
    .map_err(|e| map_verify_error(e, source_info.format))?;
    warn_if_cache_kept(&verify);
    Ok(VerifyDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        target: info,
        source: source_info,
        verify,
        ignored_ranges: ignore
            .iter()
            .map(|r| IgnoredRange {
                offset: r.start,
                length: r.end - r.start,
                reason: "gpt_repair",
            })
            .collect(),
    })
}

/// Vide le cache avant la relecture du formatage. Un échec n'arrête pas l'opération et n'est que
/// journalisé : la relecture pourrait alors porter sur le cache du système et non sur le support.
pub(super) fn drop_cache_before_verify(device: &mut dyn BlockDevice) {
    if let Err(e) = device.drop_cache() {
        log::warn!(target: "bonk::write", error = e.to_string().as_str(); "cache not dropped: the verification may read cached data");
    }
}

/// La relecture dit dans son rapport si le cache a été vidé ; le journal le signale en plus.
pub(super) fn warn_if_cache_kept(report: &engine::VerifyReport) {
    if !report.cache_dropped {
        log::warn!(target: "bonk::write", "cache not dropped: the verification may have read cached data");
    }
}

/// Plages d'une image GPT que la réparation du GPT peut réécrire (début de l'image relu à part, car le
/// flux principal est consommé par le calcul de l'empreinte). Vide si l'image est illisible : l'erreur
/// sera remontée par la lecture principale.
fn repair_ranges(image: &ImageFile) -> Vec<std::ops::Range<u64>> {
    use std::io::Read;

    let Ok((source, _)) = image.decoded() else {
        return Vec::new();
    };
    let mut head = Vec::new();
    if source.reader.take(8192).read_to_end(&mut head).is_err() {
        return Vec::new();
    }
    bonk_core::repair::ranges_rewritten_by_repair(&head)
}
