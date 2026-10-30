//! Écrire une image.

use std::path::{Path, PathBuf};

use bonk_core::BlockDevice;
use bonk_core::analysis::{self, ImageKind, ImageReport};
use bonk_core::partition::inspect;
use bonk_core::repair::{RepairReport, needs_repair, repair_gpt};
use bonk_core::write::{self as engine, Control, DEFAULT_CHUNK, Phase, Progress, VerifyError};
use bonk_hal::{Access, Elevation, Eligibility, FilterOptions, Warning, evaluate};

use super::source::ImageFile;
use super::target::{Chosen, OpenRequest, choose, open_target};
use super::translate::{guarded, map_verify_error, map_write_error};
use super::types::{
    Deps, JobError, Plan, SCHEMA_VERSION, SourceInfo, Target, TraceIo, WriteDone, WriteMode,
    WriteOutcome, fail,
};
use super::verify::warn_if_cache_kept;
use crate::errors::Code;

pub struct WriteJob {
    pub target: Target,
    pub source: PathBuf,
    pub options: FilterOptions,
    pub verify: bool,
    pub repair_gpt: bool,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    /// Empreinte SHA-256 publiée pour le fichier source, en hexadécimal.
    pub expected_sha256: Option<String>,
    pub mode: WriteMode,
    pub elevation: Elevation,
    /// Tient lieu de `yes` et autorise un disque interne déjà admis par les options du filtre. Sans
    /// aucune question : une commande doit pouvoir tourner sans personne devant l'écran.
    pub force: bool,
}

/// Contrôle le fichier source contre l'empreinte attendue. Avant tout geste : un fichier faux ne doit
/// coûter ni un démontage, ni le contenu de la clé.
fn check_sha256(
    image: &ImageFile,
    path: &Path,
    expected: &str,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(), JobError> {
    let expected = expected.trim().to_ascii_lowercase();
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(fail(
            Code::InvalidArgument,
            format!("empreinte SHA-256 invalide : « {expected} » (64 caractères hexadécimaux)"),
        ));
    }
    let mut report = guarded(deps, progress);
    let actual = engine::hash_stream(
        &mut image.raw(),
        Some(image.size()),
        DEFAULT_CHUNK,
        &mut report,
    )
    .map_err(|e| match e {
        VerifyError::Cancelled => fail(Code::Cancelled, e),
        _ => fail(Code::SourceUnreadable, format!("{} : {e}", path.display())),
    })?;
    if actual == expected {
        Ok(())
    } else {
        Err(fail(
            Code::SourceHashMismatch,
            format!("{} : attendu {expected}, calculé {actual}", path.display()),
        ))
    }
}

/// Lit le début de l'image (décompressé) et l'analyse. Rouvre le fichier : le flux principal doit rester
/// intact pour l'écriture.
fn analyze_source(
    image: &ImageFile,
    path: &Path,
    real_len: Option<u64>,
) -> Result<ImageReport, JobError> {
    use std::io::Read;

    let (source, _) = image.decoded()?;
    let mut head = Vec::new();
    source
        .reader
        .take(analysis::HEAD_LEN as u64)
        .read_to_end(&mut head)
        .map_err(|e| {
            fail(
                Code::DecompressionFailed,
                format!("{} : {e}", path.display()),
            )
        })?;
    // Quel que soit le mode : ne rien écrire et annoncer un succès serait un mensonge.
    if head.is_empty() {
        return Err(fail(
            Code::SourceUnreadable,
            format!("{} : image vide", path.display()),
        ));
    }
    Ok(analysis::analyze(&head, real_len))
}

/// Applique le mode d'écriture à l'analyse : `Raw` passe outre en le journalisant.
fn judge_image(report: &ImageReport, mode: WriteMode) -> Result<(), JobError> {
    let problem = if report.truncated == Some(true) {
        Some((
            Code::ImageTruncated,
            format!(
                "l'image annonce {} octets",
                report.declared_bytes.unwrap_or_default()
            ),
        ))
    } else if report.is_writable_as_is() {
        None
    } else if report.kind == ImageKind::PlainIso {
        let hint = if report.udf {
            " ; les ISO d'installation de Windows sont de ce type"
        } else {
            ""
        };
        Some((
            Code::ImageNotBootable,
            format!("ISO sans amorçage de disque : elle ne démarrerait pas depuis une clé{hint}"),
        ))
    } else {
        Some((
            Code::ImageNotBootable,
            "aucune structure d'amorçage ni table de partitions reconnue".to_owned(),
        ))
    };
    match (problem, mode) {
        (None, _) => Ok(()),
        (Some((code, message)), WriteMode::Auto) => Err(fail(code, message)),
        (Some((code, message)), WriteMode::Raw) => {
            log::warn!(target: "bonk::write", code = code.id(), reason = message.as_str(); "written anyway: raw mode");
            Ok(())
        }
    }
}

/// Taille que l'image occupera sur la cible, quand on la connaît avant de la lire en entier : une image
/// compressée ne donne que ce que sa structure annonce, écarté en mode `Raw` qui ne croit pas l'analyse.
fn needed_bytes(source: &SourceInfo, image: &ImageReport, mode: WriteMode) -> Option<u64> {
    source
        .size_bytes
        .or(image.declared_bytes.filter(|_| mode == WriteMode::Auto))
}

/// Une image compressée ne dit sa taille réelle qu'une fois lue : la relecture compare au flux, et ne voit
/// donc pas un flux complet dont le contenu est coupé. Contrôlé après coup, faute de mieux.
fn check_complete(image: &ImageReport, written: u64, mode: WriteMode) -> Result<(), JobError> {
    let Some(declared) = image.declared_bytes.filter(|declared| written < *declared) else {
        return Ok(());
    };
    let message = format!(
        "l'image décompressée fait {written} octets et en annonce {declared} : le support est incomplet"
    );
    match mode {
        WriteMode::Auto => Err(fail(Code::ImageTruncated, message)),
        WriteMode::Raw => {
            log::warn!(target: "bonk::write", reason = message.as_str(); "written anyway: raw mode");
            Ok(())
        }
    }
}

/// Un disque interne proposé exige `force`, et `yes` ne suffit pas : effacer un disque de la machine ne doit
/// pas partir d'une habitude de frappe. Rien n'a encore bougé : dernier contrôle avant le premier geste.
pub(super) fn confirm_internal(
    chosen: &Chosen,
    options: FilterOptions,
    force: bool,
) -> Result<(), JobError> {
    let Some(disk) = &chosen.disk else {
        return Ok(());
    };
    let internal = evaluate(disk, options)
        == (Eligibility::Offered {
            warning: Warning::InternalDisk,
        });
    if !internal {
        return Ok(());
    }
    if force {
        log::warn!(target: "bonk::write", path = disk.path.as_str(), model = disk.model.as_str(); "internal disk: all its data will be erased, --force given");
        return Ok(());
    }
    Err(fail(
        Code::InternalConfirmationRequired,
        format!(
            "{} ({}) est un disque interne : toutes ses données seraient effacées. Ajouter --force pour le confirmer",
            disk.path, disk.model
        ),
    ))
}

fn planned_steps(job: &WriteJob, chosen: &Chosen) -> Vec<&'static str> {
    let mut steps = Vec::new();
    if !chosen.mounts.is_empty() {
        steps.push("unmount");
    }
    steps.extend(["write", "flush"]);
    if job.verify {
        steps.push("verify");
    }
    if job.repair_gpt {
        steps.push("repair_gpt_if_needed");
    }
    steps
}

/// Le support est plus grand que l'image : le GPT de secours n'est plus à la fin.
fn repair_if_needed(device: &mut dyn BlockDevice) -> Result<Option<RepairReport>, JobError> {
    let layout = inspect(device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    if !needs_repair(&layout) {
        return Ok(None);
    }
    let report = repair_gpt(device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    log::info!(target: "bonk::write", actions = report.actions.join(",").as_str(); "gpt repaired");
    Ok(Some(report))
}

pub fn write(
    job: &WriteJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteOutcome, JobError> {
    let file = ImageFile::open(&job.source)?;
    let (mut source, source_info) = file.decoded()?;
    let chosen = choose(&job.target, job.options, deps)?;
    let image = analyze_source(&file, &job.source, source_info.size_bytes)?;
    judge_image(&image, job.mode)?;
    // Refus avant tout geste quand la taille de l'image est connue et dépasse la cible.
    if let (Some(needed), Some(available)) = (
        needed_bytes(&source_info, &image, job.mode),
        chosen.info.size_bytes,
    ) && needed > available
    {
        return Err(fail(
            Code::SourceTooLarge,
            format!("l'image fait {needed} octets, la cible {available} octets"),
        ));
    }

    if let Some(expected) = &job.expected_sha256 {
        check_sha256(&file, &job.source, expected, deps, progress)?;
    }
    let sha256_checked = job.expected_sha256.is_some();

    if job.dry_run {
        let steps = planned_steps(job, &chosen);
        return Ok(WriteOutcome::Plan(Box::new(Plan {
            schema_version: SCHEMA_VERSION,
            event: "plan",
            dry_run: true,
            target: chosen.info,
            source: source_info,
            image,
            sha256_checked,
            unmount: chosen.mounts,
            steps,
        })));
    }
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "écriture sans confirmation : ajouter --yes ou --force (ou décrire l'opération avec --dry-run)",
        ));
    }
    confirm_internal(&chosen, job.options, job.force)?;

    log::info!(
        target: "bonk::write",
        target_kind = chosen.info.kind, target_path = chosen.info.path.as_str(),
        source = source_info.path.as_str(), format = source_info.format.name();
        "write starting"
    );
    if chosen.disk.is_some() {
        let waiting = Progress {
            phase: Phase::Authorize,
            bytes_done: 0,
            bytes_total: source_info.size_bytes,
            bytes_per_second: 0.0,
            elapsed_ms: 0,
        };
        if guarded(deps, progress)(&waiting) == Control::Cancel {
            return Err(fail(Code::Cancelled, "annulée avant l'ouverture du disque"));
        }
    }
    let (mut device, info, unmounted) = open_target(
        chosen,
        &job.target,
        &OpenRequest {
            options: job.options,
            access: Access::ReadWrite,
            elevation: job.elevation,
            size_hint: source_info.size_bytes,
            trace_io: job.trace_io,
        },
        deps,
    )?;
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
        let report = engine::verify(
            &mut *device,
            write.bytes,
            &write.sha256,
            DEFAULT_CHUNK,
            &mut report_progress,
        )
        .map_err(|e| map_verify_error(e, source_info.format))?;
        log::info!(target: "bonk::write", bytes = report.bytes; "verification passed");
        warn_if_cache_kept(&report);
        Some(report)
    } else {
        None
    };

    let gpt_repair = if job.repair_gpt {
        repair_if_needed(&mut *device)?
    } else {
        None
    };
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
    })))
}
