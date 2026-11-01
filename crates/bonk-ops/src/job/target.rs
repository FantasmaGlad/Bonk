//! Choix et ouverture de la cible, les garde-fous avant le premier geste ; relecture de sa table après le
//! dernier.

use std::fs;
use std::path::Path;

use bonk_core::trace::TracedDevice;
use bonk_core::{BlockDevice, BlockError, DEFAULT_BLOCK_SIZE};
use bonk_hal::{
    Access, Disk, Elevation, FileDevice, FilterOptions, OpenTargetError, TargetIdentity,
    mount_points, reverify,
};

use super::types::{Deps, JobError, Target, TargetInfo, TraceIo, fail};
use crate::errors::Code;

/// Cible ouverte : périphérique, informations et points démontés.
pub(super) type OpenedTarget = (Box<dyn BlockDevice>, TargetInfo, Vec<String>);

/// Cible reconnue : informations, disque s'il y en a un, et ce qui sera démonté. Son identité est
/// fixée **une fois**, au choix : c'est elle que l'on revérifie avant d'ouvrir, jamais celle d'une
/// énumération plus récente (sinon un disque remplacé entre-temps serait accepté sans le dire).
pub(super) struct Chosen {
    pub(super) info: TargetInfo,
    pub(super) disk: Option<Disk>,
    pub(super) mounts: Vec<String>,
}

pub(super) fn map_open_target(err: OpenTargetError) -> JobError {
    let code = match &err {
        OpenTargetError::NotFound(_) => Code::DiskNotFound,
        OpenTargetError::Refused(_) => Code::TargetRefused,
        OpenTargetError::IdentityChanged { .. } => Code::IdentityChanged,
        OpenTargetError::Unmount(_) => Code::UnmountFailed,
    };
    fail(code, err)
}

pub(super) fn enumerate(deps: &Deps<'_>) -> Result<Vec<Disk>, JobError> {
    (deps.enumerate)().map_err(|e| fail(Code::of_enumeration(&e), e))
}

pub(super) fn choose(
    target: &Target,
    options: FilterOptions,
    deps: &Deps<'_>,
) -> Result<Chosen, JobError> {
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
                        format!("{} is not a regular image file", path.display()),
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

/// Fait relire au système la table du disque qui vient d'être écrit, une fois le disque refermé : le service
/// ne relit pas la table d'un disque ouvert en exclusif. Sans cela, le noyau peut garder l'ancienne taille
/// d'une partition, et la lire sur cette machine échoue jusqu'au prochain branchement. Un échec n'enlève
/// rien à ce qui est écrit : il est signalé, pas fatal.
pub(super) fn reread_table(disk: &Path, elevation: Elevation, deps: &Deps<'_>) {
    if let Err(error) = (deps.io.rescan)(disk, elevation) {
        log::warn!(
            target: "bonk::write",
            error = error.to_string().as_str();
            "partition table not reread: unplug and replug the disk"
        );
    }
}

/// Comment ouvrir la cible.
pub(super) struct OpenRequest {
    pub(super) options: FilterOptions,
    pub(super) access: Access,
    pub(super) elevation: Elevation,
    pub(super) size_hint: Option<u64>,
    pub(super) trace_io: Option<TraceIo>,
}

/// Choisit et ouvre la cible en lecture seule : rien à démonter, rien à écrire.
pub(super) fn open_read_only(
    target: &Target,
    options: FilterOptions,
    elevation: Elevation,
    trace_io: Option<TraceIo>,
    deps: &Deps<'_>,
) -> Result<(Box<dyn BlockDevice>, TargetInfo), JobError> {
    let chosen = choose(target, options, deps)?;
    let (device, info, _) = open_target(
        chosen,
        target,
        &OpenRequest {
            options,
            access: Access::ReadOnly,
            elevation,
            size_hint: None,
            trace_io,
        },
        deps,
    )?;
    Ok((device, info))
}

/// Ouvre la cible choisie et rend le périphérique, ses informations et les points démontés.
pub(super) fn open_target(
    chosen: Chosen,
    target: &Target,
    request: &OpenRequest,
    deps: &Deps<'_>,
) -> Result<OpenedTarget, JobError> {
    let &OpenRequest {
        options,
        access,
        elevation,
        size_hint,
        trace_io,
    } = request;
    let writing = access == Access::ReadWrite;
    let (device, unmounted): (Box<dyn BlockDevice>, Vec<String>) = match (&chosen.disk, target) {
        (Some(disk), _) => {
            let identity = TargetIdentity::from(disk);
            // Le droit d'abord : un refus ne doit laisser la clé ni démontée ni touchée.
            let prepared = (deps.io.prepare)(Path::new(&disk.path), access, elevation);
            // Annulé pendant l'attente de l'autorisation : la clé reste montée, intacte.
            if (deps.io.cancelled)() {
                return Err(fail(Code::Cancelled, "cancelled before opening the disk"));
            }
            prepared.map_err(open_error)?;
            // Démonter seulement pour écrire : relire un volume monté ne le perturbe pas.
            let unmounted = if writing {
                (deps.io.unmount)(disk, elevation).map_err(map_open_target)?
            } else {
                Vec::new()
            };
            // Identité revérifiée immédiatement avant d'ouvrir.
            let fresh = enumerate(deps)?;
            let again = reverify(&identity, &fresh, options).map_err(map_open_target)?;
            let opened = (deps.io.open_node)(Path::new(&again.path), access, elevation);
            if (deps.io.cancelled)() {
                return Err(fail(Code::Cancelled, "cancelled before opening the disk"));
            }
            let node = opened.map_err(open_error)?;
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
                FileDevice::open(path, access, DEFAULT_BLOCK_SIZE).map_err(open_error)?
            } else if writing {
                let wanted = size.or(size_hint).ok_or_else(|| {
                    fail(
                        Code::InvalidArgument,
                        "target image size unknown: give it with --size",
                    )
                })?;
                FileDevice::create(
                    path,
                    wanted.div_ceil(u64::from(DEFAULT_BLOCK_SIZE)) * u64::from(DEFAULT_BLOCK_SIZE),
                    DEFAULT_BLOCK_SIZE,
                )
                .map_err(open_error)?
            } else {
                return Err(fail(
                    Code::SourceUnreadable,
                    format!("{}: file missing", path.display()),
                ));
            };
            (Box::new(device) as Box<dyn BlockDevice>, Vec::new())
        }
        (None, Target::Disk(_)) => unreachable!("a disk target always has a chosen disk"),
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
