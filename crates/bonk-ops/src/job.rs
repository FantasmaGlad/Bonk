//! Les opérations d'écriture : écrire, vérifier, réparer le GPT (CDC §3, §4, J1).
//!
//! Ce module est la porte d'écriture unique, partagée par la ligne de commande, le service
//! (`bonk serve`) et tout autre front : il enchaîne les garde-fous dans l'ordre voulu et ne parle jamais à l'utilisateur. Il rend
//! des résultats typés (et sérialisables, d'où `bonk schema`) ou une [`JobError`] qui porte son code.
//!
//! Ordre d'une écriture sur un disque : le filtre juge la cible ; sans `--yes` rien ne bouge ; les
//! partitions sont démontées ; **juste avant l'ouverture** le disque est ré-énuméré et son identité
//! revérifiée (CDC §3.3) ; le noyau refuse enfin un disque encore utilisé (`O_EXCL`). Les tests
//! injectent ces dépendances ([`Deps`]) et n'ouvrent jamais de vrai disque.

use std::fmt::Display;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use bonk_core::codec::{self, CodecError, Format, Source};
use bonk_core::partition::inspect;
use bonk_core::repair::{RepairError, RepairReport, needs_repair, repair_gpt};
use bonk_core::trace::TracedDevice;
use bonk_core::write::{
    self as engine, Control, DEFAULT_CHUNK, Progress, VerifyError, VerifyReport, WriteError,
    WriteReport,
};
use bonk_core::{BlockDevice, BlockError};
use bonk_hal::{
    Access, Disk, EnumerateError, FileDevice, FilterOptions, OpenTargetError, TargetIdentity,
    mount_points, reverify,
};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;

/// Version des formats de résultat, incrémentée à chaque changement incompatible.
pub const SCHEMA_VERSION: u32 = 1;

/// Niveau de détail de `--trace-io`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceIo {
    /// Décalage, longueur, durée et somme de contrôle de chaque accès.
    Metadata,
    /// Avec les 16 premiers octets : réservé aux fichiers image.
    Data,
}

/// Une erreur d'opération, avec son code stable.
#[derive(Debug)]
pub struct JobError {
    pub code: Code,
    pub message: String,
}

fn fail(code: Code, message: impl Display) -> JobError {
    JobError {
        code,
        message: message.to_string(),
    }
}

/// Démontage des partitions d'un disque : rend les points démontés.
pub type UnmountFn = Box<dyn Fn(&Disk) -> Result<Vec<String>, OpenTargetError>>;
/// Ouverture d'un disque entier en accès brut.
pub type OpenNodeFn = Box<dyn Fn(&Path, Access) -> Result<Box<dyn BlockDevice>, BlockError>>;
/// Cible ouverte : périphérique, informations et points démontés.
type OpenedTarget = (Box<dyn BlockDevice>, TargetInfo, Vec<String>);

/// Ce que les opérations demandent au système : injectable pour les tests.
pub struct Io {
    pub unmount: UnmountFn,
    pub open_node: OpenNodeFn,
    pub cancelled: Box<dyn Fn() -> bool>,
}

impl Default for Io {
    fn default() -> Self {
        Self::with_cancel(None)
    }
}

impl Io {
    /// Accès système réels. `cancel` est un drapeau d'annulation propre à l'opération (le service en
    /// donne un par opération) ; Ctrl+C annule dans tous les cas.
    pub fn with_cancel(cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>) -> Self {
        Self {
            unmount: Box::new(|disk| bonk_hal::platform::current().unmount(disk)),
            open_node: Box::new(|path, access| {
                bonk_hal::platform::current().open_disk(path, access)
            }),
            cancelled: Box::new(move || {
                bonk_hal::interrupt::requested()
                    || cancel
                        .as_ref()
                        .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::SeqCst))
            }),
        }
    }
}

/// Dépendances d'une opération : énumération des disques et accès système.
pub struct Deps<'a> {
    pub enumerate: &'a dyn Fn() -> Result<Vec<Disk>, EnumerateError>,
    pub io: &'a Io,
}

/// Où écrire.
#[derive(Debug, Clone)]
pub enum Target {
    /// Un disque, par son identifiant stable (jamais `/dev/sdX`).
    Disk(String),
    /// Un fichier image (mode `--simulate`), créé à `size` octets s'il n'existe pas.
    Image { path: PathBuf, size: Option<u64> },
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct TargetInfo {
    /// `disk` ou `image`.
    pub kind: &'static str,
    pub id: Option<String>,
    pub path: String,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SourceInfo {
    pub path: String,
    pub format: Format,
    /// Taille connue d'avance (image brute seulement).
    pub size_bytes: Option<u64>,
}

/// Ce que fera une écriture, sans rien faire (`--dry-run`).
#[derive(Debug, Serialize, JsonSchema)]
pub struct Plan {
    pub schema_version: u32,
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub source: SourceInfo,
    /// Points de montage qui seront démontés.
    pub unmount: Vec<String>,
    pub steps: Vec<&'static str>,
}

/// Résultat d'une écriture.
#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub unmounted: Vec<String>,
    pub write: WriteReport,
    pub verify: Option<VerifyReport>,
    pub gpt_repair: Option<RepairReport>,
}

/// Résultat d'une vérification sans écriture.
#[derive(Debug, Serialize, JsonSchema)]
pub struct VerifyDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub target: TargetInfo,
    pub source: SourceInfo,
    pub verify: VerifyReport,
    /// Octets exclus de la comparaison (`sha256` est alors l'empreinte de l'image avec ces octets remis
    /// à zéro). Vide pour une image sans GPT ou aussi grande que le support.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored_ranges: Vec<IgnoredRange>,
}

/// Plage d'octets exclue d'une comparaison, avec la raison.
#[derive(Debug, Serialize, JsonSchema)]
pub struct IgnoredRange {
    pub offset: u64,
    pub length: u64,
    /// `gpt_repair` : champs que la réparation du GPT réécrit sur un support plus grand que l'image.
    pub reason: &'static str,
}

/// Résultat (ou plan) d'une réparation du GPT.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RepairDone {
    pub schema_version: u32,
    pub event: &'static str,
    pub dry_run: bool,
    pub target: TargetInfo,
    pub needs_repair: bool,
    /// Codes des constats avant et après.
    pub findings_before: Vec<String>,
    pub repair: Option<RepairReport>,
    pub findings_after: Option<Vec<String>>,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum WriteOutcome {
    Plan(Box<Plan>),
    Done(Box<WriteDone>),
}

// ---------------------------------------------------------------------------------------------
// Source
// ---------------------------------------------------------------------------------------------

pub fn open_source(path: &Path) -> Result<(Source, SourceInfo), JobError> {
    let metadata = fs::metadata(path)
        .map_err(|e| fail(Code::SourceUnreadable, format!("{} : {e}", path.display())))?;
    if !metadata.is_file() {
        return Err(fail(
            Code::SourceUnreadable,
            format!("{} n'est pas un fichier ordinaire", path.display()),
        ));
    }
    let file = File::open(path)
        .map_err(|e| fail(Code::SourceUnreadable, format!("{} : {e}", path.display())))?;
    let source = codec::open(file, Some(metadata.len())).map_err(|e| match e {
        CodecError::Unsupported(_) => fail(Code::SourceFormatUnsupported, e),
        CodecError::Io(_) => fail(Code::SourceUnreadable, format!("{} : {e}", path.display())),
    })?;
    let info = SourceInfo {
        path: path.display().to_string(),
        format: source.format,
        size_bytes: source.output_len,
    };
    Ok((source, info))
}

// ---------------------------------------------------------------------------------------------
// Cible
// ---------------------------------------------------------------------------------------------

/// Cible reconnue : informations, disque s'il y en a un, et ce qui sera démonté. Son identité est
/// fixée **une fois**, au choix : c'est elle que l'on revérifie avant d'ouvrir, jamais celle d'une
/// énumération plus récente (sinon un disque remplacé entre-temps serait accepté sans le dire).
struct Chosen {
    info: TargetInfo,
    disk: Option<Disk>,
    mounts: Vec<String>,
}

fn map_open_target(err: OpenTargetError) -> JobError {
    let code = match &err {
        OpenTargetError::NotFound(_) => Code::DiskNotFound,
        OpenTargetError::Refused(_) => Code::TargetRefused,
        OpenTargetError::IdentityChanged { .. } => Code::IdentityChanged,
        OpenTargetError::Unmount(_) => Code::UnmountFailed,
    };
    fail(code, err)
}

fn enumerate(deps: &Deps<'_>) -> Result<Vec<Disk>, JobError> {
    (deps.enumerate)().map_err(|e| fail(Code::of_enumeration(&e), e))
}

fn choose(target: &Target, options: FilterOptions, deps: &Deps<'_>) -> Result<Chosen, JobError> {
    match target {
        Target::Disk(id) => {
            let disks = enumerate(deps)?;
            let found = disks
                .iter()
                .find(|d| d.id == *id)
                .ok_or_else(|| map_open_target(OpenTargetError::NotFound(id.clone())))?;
            let identity = TargetIdentity::from(found);
            let disk = reverify(&identity, &disks, options).map_err(map_open_target)?;
            Ok(Chosen {
                info: TargetInfo {
                    kind: "disk",
                    id: Some(disk.id.clone()),
                    path: disk.path.clone(),
                    size_bytes: Some(disk.size_bytes),
                },
                mounts: mount_points(disk),
                disk: Some(disk.clone()),
            })
        }
        Target::Image { path, size } => {
            let size_bytes = match fs::metadata(path) {
                Ok(m) if m.is_file() => Some(m.len()),
                Ok(_) => {
                    return Err(fail(
                        Code::SourceUnreadable,
                        format!("{} n'est pas un fichier image ordinaire", path.display()),
                    ));
                }
                Err(_) => *size,
            };
            Ok(Chosen {
                info: TargetInfo {
                    kind: "image",
                    id: None,
                    path: path.display().to_string(),
                    size_bytes,
                },
                disk: None,
                mounts: Vec::new(),
            })
        }
    }
}

fn open_error(err: BlockError) -> JobError {
    match &err {
        BlockError::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            fail(Code::DeviceOpenDenied, err)
        }
        _ => fail(Code::WriteFailed, err),
    }
}

/// Ouvre la cible choisie et rend le périphérique, ses informations et les points démontés.
fn open_target(
    chosen: Chosen,
    target: &Target,
    options: FilterOptions,
    access: Access,
    size_hint: Option<u64>,
    trace_io: Option<TraceIo>,
    deps: &Deps<'_>,
) -> Result<OpenedTarget, JobError> {
    let writing = access == Access::ReadWrite;
    let (device, unmounted): (Box<dyn BlockDevice>, Vec<String>) = match (&chosen.disk, target) {
        (Some(disk), _) => {
            let identity = TargetIdentity::from(disk);
            // Démonter seulement pour écrire : relire un volume monté ne le perturbe pas.
            let unmounted = if writing {
                (deps.io.unmount)(disk).map_err(map_open_target)?
            } else {
                Vec::new()
            };
            // CDC §3.3 : identité revérifiée immédiatement avant d'ouvrir.
            let fresh = enumerate(deps)?;
            let again = reverify(&identity, &fresh, options).map_err(map_open_target)?;
            let node = (deps.io.open_node)(Path::new(&again.path), access).map_err(open_error)?;
            if node.size() != identity.size_bytes {
                return Err(map_open_target(OpenTargetError::IdentityChanged {
                    expected: identity.clone(),
                    actual: TargetIdentity {
                        size_bytes: node.size(),
                        ..identity
                    },
                }));
            }
            (node, unmounted)
        }
        (None, Target::Image { path, size }) => {
            let device = if path.exists() {
                FileDevice::open(path, access, 512).map_err(open_error)?
            } else if writing {
                let wanted = size.or(size_hint).ok_or_else(|| {
                    fail(
                        Code::InvalidArgument,
                        "taille de l'image cible inconnue : la donner avec --size",
                    )
                })?;
                FileDevice::create(path, wanted.div_ceil(512) * 512, 512).map_err(open_error)?
            } else {
                return Err(fail(
                    Code::SourceUnreadable,
                    format!("{} : fichier absent", path.display()),
                ));
            };
            (Box::new(device) as Box<dyn BlockDevice>, Vec::new())
        }
        (None, Target::Disk(_)) => unreachable!("une cible disque a toujours un disque choisi"),
    };
    let device: Box<dyn BlockDevice> = match trace_io {
        Some(mode) => {
            // Les données d'un disque réel ne sont jamais journalisées, même sur demande.
            let head = mode == TraceIo::Data && chosen.disk.is_none();
            if mode == TraceIo::Data && !head {
                log::warn!(target: "bonk::write", "--trace-io=data ignored: reserved for image files");
            }
            Box::new(TracedDevice::new(device, head))
        }
        None => device,
    };
    let mut info = chosen.info;
    info.size_bytes = Some(device.size());
    Ok((device, info, unmounted))
}

// ---------------------------------------------------------------------------------------------
// Traduction des erreurs du moteur
// ---------------------------------------------------------------------------------------------

fn source_error(format: Format, err: &std::io::Error) -> Code {
    if format == Format::Raw {
        let _ = err;
        Code::SourceUnreadable
    } else {
        Code::DecompressionFailed
    }
}

fn map_write_error(err: WriteError, format: Format) -> JobError {
    let code = match &err {
        WriteError::Cancelled { .. } => Code::Cancelled,
        WriteError::TooLarge { .. } => Code::SourceTooLarge,
        WriteError::Source(io) => source_error(format, io),
        WriteError::Device(BlockError::Io(io))
            if io.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            Code::DeviceOpenDenied
        }
        WriteError::Device(_) => Code::WriteFailed,
    };
    fail(code, err)
}

fn map_verify_error(err: VerifyError, format: Format) -> JobError {
    let code = match &err {
        VerifyError::Mismatch { .. } => Code::VerifyMismatch,
        VerifyError::Cancelled => Code::Cancelled,
        VerifyError::Source(io) => source_error(format, io),
        VerifyError::Device(_) => Code::WriteFailed,
    };
    fail(code, err)
}

/// Enveloppe le rappel de progression : l'annulation demandée par le système (Ctrl+C) l'emporte.
fn guarded<'a>(
    deps: &'a Deps<'a>,
    progress: &'a mut dyn FnMut(&Progress) -> Control,
) -> impl FnMut(&Progress) -> Control + 'a {
    move |p| {
        if (deps.io.cancelled)() {
            Control::Cancel
        } else {
            progress(p)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Écrire
// ---------------------------------------------------------------------------------------------

pub struct WriteJob {
    pub target: Target,
    pub source: PathBuf,
    pub options: FilterOptions,
    pub verify: bool,
    pub repair_gpt: bool,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn write(
    job: &WriteJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<WriteOutcome, JobError> {
    let (mut source, source_info) = open_source(&job.source)?;
    let chosen = choose(&job.target, job.options, deps)?;
    // Refus avant tout geste quand la taille de l'image est connue et dépasse la cible.
    if let (Some(needed), Some(available)) = (source_info.size_bytes, chosen.info.size_bytes)
        && needed > available
    {
        return Err(fail(
            Code::SourceTooLarge,
            format!("l'image fait {needed} octets, la cible {available} octets"),
        ));
    }

    if job.dry_run {
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
        return Ok(WriteOutcome::Plan(Box::new(Plan {
            schema_version: SCHEMA_VERSION,
            event: "plan",
            dry_run: true,
            target: chosen.info,
            source: source_info,
            unmount: chosen.mounts,
            steps,
        })));
    }
    if !job.yes {
        return Err(fail(
            Code::ConfirmationRequired,
            "écriture sans confirmation : ajouter --yes (ou décrire l'opération avec --dry-run)",
        ));
    }

    log::info!(
        target: "bonk::write",
        target_kind = chosen.info.kind, target_path = chosen.info.path.as_str(),
        source = source_info.path.as_str(), format = source_info.format.name();
        "write starting"
    );
    let (mut device, info, unmounted) = open_target(
        chosen,
        &job.target,
        job.options,
        Access::ReadWrite,
        source_info.size_bytes,
        job.trace_io,
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

    let verify = if job.verify {
        // Sans cela, la relecture lirait le cache du noyau (ce que Bonk vient d'écrire) et non le support.
        drop_cache_before_verify(&mut *device);
        let report = engine::verify(
            &*device,
            write.bytes,
            &write.sha256,
            DEFAULT_CHUNK,
            &mut report_progress,
        )
        .map_err(|e| map_verify_error(e, source_info.format))?;
        log::info!(target: "bonk::write", bytes = report.bytes; "verification passed");
        Some(report)
    } else {
        None
    };

    let gpt_repair = if job.repair_gpt {
        let layout = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
        if needs_repair(&layout) {
            let report = repair_gpt(&mut *device).map_err(|e| fail(Code::GptRepairFailed, e))?;
            log::info!(target: "bonk::write", actions = report.actions.join(",").as_str(); "gpt repaired");
            Some(report)
        } else {
            None
        }
    } else {
        None
    };
    Ok(WriteOutcome::Done(Box::new(WriteDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        target: info,
        source: source_info,
        unmounted,
        write,
        verify,
        gpt_repair,
    })))
}

// ---------------------------------------------------------------------------------------------
// Vérifier
// ---------------------------------------------------------------------------------------------

pub struct VerifyJob {
    pub target: Target,
    pub source: PathBuf,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
}

pub fn verify(
    job: &VerifyJob,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<VerifyDone, JobError> {
    let (mut source, source_info) = open_source(&job.source)?;
    let chosen = choose(&job.target, job.options, deps)?;
    let (mut device, info, _) = open_target(
        chosen,
        &job.target,
        job.options,
        Access::ReadOnly,
        None,
        job.trace_io,
        deps,
    )?;
    let mut report_progress = guarded(deps, progress);
    drop_cache_before_verify(&mut *device);
    // Une image GPT posée sur un support plus grand a vu quelques champs réécrits par la réparation du
    // GPT (MBR de protection, positions de l'en-tête) : les comparer à l'image donnerait un faux échec.
    let ignore = if source_info.size_bytes.is_none_or(|n| n < device.size()) {
        repair_ranges(&job.source)
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
                "l'image fait {len} octets, la cible {} octets",
                device.size()
            ),
        ));
    }
    let verify = engine::verify_ignoring(
        &*device,
        len,
        &sha,
        DEFAULT_CHUNK,
        &ignore,
        &mut report_progress,
    )
    .map_err(|e| map_verify_error(e, source_info.format))?;
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

/// Vide le cache avant une relecture de contrôle. Un échec n'arrête pas l'opération mais est signalé :
/// la comparaison pourrait alors porter sur le cache du système et non sur le support.
fn drop_cache_before_verify(device: &mut dyn BlockDevice) {
    if let Err(e) = device.drop_cache() {
        log::warn!(target: "bonk::write", error = e.to_string().as_str(); "cache not dropped: the verification may read cached data");
    }
}

/// Plages d'une image GPT que la réparation du GPT peut réécrire (début de l'image relu à part, car le
/// flux principal est consommé par le calcul de l'empreinte). Vide si l'image est illisible : l'erreur
/// sera remontée par la lecture principale.
fn repair_ranges(path: &Path) -> Vec<std::ops::Range<u64>> {
    use std::io::Read;

    let Ok((source, _)) = open_source(path) else {
        return Vec::new();
    };
    let mut head = Vec::new();
    if source.reader.take(8192).read_to_end(&mut head).is_err() {
        return Vec::new();
    }
    bonk_core::repair::ranges_rewritten_by_repair(&head)
}

// ---------------------------------------------------------------------------------------------
// Réparer le GPT
// ---------------------------------------------------------------------------------------------

pub struct RepairJob {
    pub target: Target,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn repair(job: &RepairJob, deps: &Deps<'_>) -> Result<RepairDone, JobError> {
    let codes = |layout: &bonk_core::partition::Layout| -> Vec<String> {
        layout.findings.iter().map(|f| f.code.to_owned()).collect()
    };
    if job.dry_run || !job.yes {
        if !job.dry_run {
            return Err(fail(
                Code::ConfirmationRequired,
                "réparation sans confirmation : ajouter --yes (ou décrire avec --dry-run)",
            ));
        }
        let chosen = choose(&job.target, job.options, deps)?;
        let (device, info, _) = open_target(
            chosen,
            &job.target,
            job.options,
            Access::ReadOnly,
            None,
            job.trace_io,
            deps,
        )?;
        let layout = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
        return Ok(RepairDone {
            schema_version: SCHEMA_VERSION,
            event: "plan",
            dry_run: true,
            target: info,
            needs_repair: needs_repair(&layout),
            findings_before: codes(&layout),
            repair: None,
            findings_after: None,
        });
    }
    let chosen = choose(&job.target, job.options, deps)?;
    let (mut device, info, _) = open_target(
        chosen,
        &job.target,
        job.options,
        Access::ReadWrite,
        None,
        job.trace_io,
        deps,
    )?;
    let before = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    let needed = needs_repair(&before);
    let repair = if needed {
        Some(repair_gpt(&mut *device).map_err(|e| {
            let code = match e {
                RepairError::Device(_) => Code::WriteFailed,
                _ => Code::GptRepairFailed,
            };
            fail(code, e)
        })?)
    } else {
        None
    };
    let after = inspect(&*device).map_err(|e| fail(Code::GptRepairFailed, e))?;
    Ok(RepairDone {
        schema_version: SCHEMA_VERSION,
        event: "result",
        dry_run: false,
        target: info,
        needs_repair: needed,
        findings_before: codes(&before),
        repair,
        findings_after: Some(codes(&after)),
    })
}
