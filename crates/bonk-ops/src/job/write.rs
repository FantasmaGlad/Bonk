//! Écrire une image.

use std::path::{Path, PathBuf};

use bonk_core::BlockDevice;
use bonk_core::analysis::{self, ImageKind, ImageReport};
use bonk_core::codec::Format;
use bonk_core::format::table::TableKind;
use bonk_core::partition::inspect;
use bonk_core::repair::{RepairReport, needs_repair, repair_gpt};
use bonk_core::write::{self as engine, Control, DEFAULT_CHUNK, Phase, Progress, VerifyError};
use bonk_hal::{Access, Elevation, Eligibility, FilterOptions, Warning, evaluate};

use super::extract::Extraction;
use super::extract::{self, WindowsImage};
use super::source::ImageFile;
use super::target::{Chosen, OpenRequest, choose, open_target, reread_table};
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
    /// Table de la clé Windows ; sans effet sur une écriture brute, dont la table vient de l'image.
    pub table: TableKind,
    pub elevation: Elevation,
    /// Tient lieu de `yes` et autorise un disque interne déjà admis par les options du filtre. Sans
    /// aucune question : une commande doit pouvoir tourner sans personne devant l'écran.
    pub force: bool,
}

/// Empreinte SHA-256 en minuscules, ou le refus de ce qui n'en est pas une.
///
/// # Errors
/// `InvalidArgument` si ce ne sont pas 64 caractères hexadécimaux.
pub fn normalize_sha256(text: &str) -> Result<String, JobError> {
    let hex = text.trim().to_ascii_lowercase();
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(hex)
    } else {
        Err(fail(
            Code::InvalidArgument,
            format!("invalid SHA-256 hash: '{hex}' (64 hexadecimal characters)"),
        ))
    }
}

/// Ce qui refuserait l'écriture quelle que soit l'image : à vérifier avant un long téléchargement, qui ne
/// doit pas finir sur un refus déjà connu. Ne touche à rien.
///
/// # Errors
/// Cible refusée, confirmation absente.
pub fn precheck(job: &WriteJob, deps: &Deps<'_>) -> Result<(), JobError> {
    let chosen = choose(&job.target, job.options, deps)?;
    if job.dry_run {
        return Ok(());
    }
    confirm(job, &chosen)
}

fn confirm(job: &WriteJob, chosen: &Chosen) -> Result<(), JobError> {
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "write without confirmation: add --yes or --force (or describe the operation with --dry-run)",
        ));
    }
    confirm_internal(chosen, job.options, job.force)
}

/// Décompresse l'image une fois à blanc. Un flux décompressé ne montre sa fin qu'en dernier : sans ce
/// passage, des données après le flux ou une image plus grande que la cible ne seraient vues qu'une fois
/// la clé effacée. Une empreinte contrôlée rend ce passage inutile.
fn rehearse(
    image_file: &ImageFile,
    image: &ImageReport,
    available: Option<u64>,
    mode: WriteMode,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<(), JobError> {
    let (mut source, info) = image_file.decoded()?;
    let mut report = guarded(deps, progress);
    let (len, _) =
        engine::hash_source_ignoring(&mut source.reader, None, DEFAULT_CHUNK, &[], &mut report)
            .map_err(|e| map_verify_error(e, info.format))?;
    if let Some(available) = available.filter(|available| len > *available) {
        return Err(fail(
            Code::SourceTooLarge,
            format!("the decompressed image is {len} bytes, the target {available} bytes"),
        ));
    }
    check_complete(image, len, mode)
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
    let expected = normalize_sha256(expected)?;
    let mut report = guarded(deps, progress);
    let actual = engine::hash_stream(
        &mut image.raw(),
        Some(image.size()),
        DEFAULT_CHUNK,
        &mut report,
    )
    .map_err(|e| match e {
        VerifyError::Cancelled => fail(Code::Cancelled, e),
        _ => fail(Code::SourceUnreadable, format!("{}: {e}", path.display())),
    })?;
    if actual == expected {
        Ok(())
    } else {
        Err(fail(
            Code::SourceHashMismatch,
            format!("{}: expected {expected}, computed {actual}", path.display()),
        ))
    }
}

/// Lit le début de l'image (décompressé) et l'analyse, par un lecteur à part : le flux principal doit
/// rester intact pour l'écriture.
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
                format!("{}: {e}", path.display()),
            )
        })?;
    // Quel que soit le mode : ne rien écrire et annoncer un succès serait un mensonge.
    if head.is_empty() {
        return Err(fail(
            Code::SourceUnreadable,
            format!("{}: empty image", path.display()),
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
                "the image declares {} bytes",
                report.declared_bytes.unwrap_or_default()
            ),
        ))
    } else if report.is_writable_as_is() {
        None
    } else if report.kind == ImageKind::PlainIso {
        let hint = if report.udf {
            "; Bonk does not recognize a Windows 10 or 11 x86_64 installation in it"
        } else {
            ""
        };
        Some((
            Code::ImageNotBootable,
            format!("ISO without disk boot: it would not boot from a USB drive{hint}"),
        ))
    } else {
        Some((
            Code::ImageNotBootable,
            "no boot structure or partition table recognized".to_owned(),
        ))
    };
    match (problem, mode) {
        (None, _) => Ok(()),
        (Some((code, message)), WriteMode::Auto | WriteMode::Extract) => Err(fail(code, message)),
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
        "the decompressed image is {written} bytes and declares {declared}: the drive is incomplete"
    );
    match mode {
        WriteMode::Auto | WriteMode::Extract => Err(fail(Code::ImageTruncated, message)),
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
            "{} ({}) is an internal disk: all its data would be erased. Add --force to confirm",
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
    if chosen.disk.is_some() {
        steps.push("reread_table");
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

/// L'ISO d'installation de Windows que le mode demande d'extraire, s'il y en a une. `extract` exige d'en
/// trouver une ; `auto` en cherche une là où l'écriture brute serait refusée ; `raw` n'en cherche jamais.
fn windows_image(
    file: &ImageFile,
    source: &SourceInfo,
    image: &ImageReport,
    mode: WriteMode,
) -> Result<Option<WindowsImage>, JobError> {
    let wanted = match mode {
        WriteMode::Raw => false,
        WriteMode::Extract => true,
        WriteMode::Auto => image.udf && !image.is_writable_as_is(),
    };
    if !wanted || source.format != Format::Raw {
        return if mode == WriteMode::Extract {
            Err(fail(
                Code::ImageNotBootable,
                "--mode extract requires an uncompressed Windows installation ISO",
            ))
        } else {
            Ok(None)
        };
    }
    let found = extract::detect(file)?;
    if found.is_none() && mode == WriteMode::Extract {
        return Err(fail(
            Code::ImageNotBootable,
            "this is not a Windows 10 or 11 x86_64 installation ISO (UDF, sources/install.wim, efi/boot/bootx64.efi)",
        ));
    }
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
fn write_windows(
    windows: &WindowsImage,
    file: &ImageFile,
    source: SourceInfo,
    image: ImageReport,
    chosen: Chosen,
    job: &WriteJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteOutcome, JobError> {
    windows.check_fits(chosen.info.size_bytes, job.table)?;
    if let Some(expected) = &job.expected_sha256 {
        check_sha256(file, &job.source, expected, deps, progress)?;
    }
    let sha256_checked = job.expected_sha256.is_some();
    if job.dry_run {
        let mut steps = vec!["unmount", "write_table", "write_fat32", "copy_files"];
        if chosen.disk.is_none() {
            steps.remove(0);
        }
        steps.push("verify_files");
        if chosen.disk.is_some() {
            steps.push("reread_table");
        }
        return Ok(plan(
            chosen,
            source,
            image,
            sha256_checked,
            steps,
            Some(windows.describe(job.table)),
        ));
    }
    confirm(job, &chosen)?;
    extract::run(windows, file, chosen, job, sha256_checked, deps, progress)
        .map(|done| WriteOutcome::Extracted(Box::new(done)))
}

/// Le plan d'un essai à blanc, le même pour une image brute et une clé Windows.
fn plan(
    chosen: Chosen,
    source: SourceInfo,
    image: ImageReport,
    sha256_checked: bool,
    steps: Vec<&'static str>,
    extraction: Option<Extraction>,
) -> WriteOutcome {
    WriteOutcome::Plan(Box::new(Plan {
        schema_version: SCHEMA_VERSION,
        event: "plan",
        dry_run: true,
        target: chosen.info,
        source,
        image,
        sha256_checked,
        unmount: chosen.mounts,
        steps,
        extraction,
    }))
}

/// L'autorisation du système peut faire attendre : le dire, et laisser annuler avant d'ouvrir le disque.
fn announce_authorization(
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

/// Refus avant tout geste quand la taille de l'image est connue et dépasse la cible.
fn refuse_if_too_large(needed: Option<u64>, available: Option<u64>) -> Result<(), JobError> {
    if let (Some(needed), Some(available)) = (needed, available)
        && needed > available
    {
        return Err(fail(
            Code::SourceTooLarge,
            format!("the image is {needed} bytes, the target {available} bytes"),
        ));
    }
    Ok(())
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
    if let Some(windows) = windows_image(&file, &source_info, &image, job.mode)? {
        return write_windows(
            &windows,
            &file,
            source_info,
            image,
            chosen,
            job,
            deps,
            progress,
        );
    }
    judge_image(&image, job.mode)?;
    refuse_if_too_large(
        needed_bytes(&source_info, &image, job.mode),
        chosen.info.size_bytes,
    )?;

    if let Some(expected) = &job.expected_sha256 {
        check_sha256(&file, &job.source, expected, deps, progress)?;
    }
    let sha256_checked = job.expected_sha256.is_some();

    if job.dry_run {
        let steps = planned_steps(job, &chosen);
        return Ok(plan(
            chosen,
            source_info,
            image,
            sha256_checked,
            steps,
            None,
        ));
    }
    confirm(job, &chosen)?;
    if !sha256_checked && source_info.format != Format::Raw {
        rehearse(
            &file,
            &image,
            chosen.info.size_bytes,
            job.mode,
            deps,
            progress,
        )?;
    }

    log::info!(
        target: "bonk::write",
        target_kind = chosen.info.kind, target_path = chosen.info.path.as_str(),
        source = source_info.path.as_str(), format = source_info.format.name();
        "write starting"
    );
    if chosen.disk.is_some() {
        announce_authorization(source_info.size_bytes, deps, progress)?;
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
    })))
}
