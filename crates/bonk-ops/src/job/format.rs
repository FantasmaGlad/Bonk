//! Formater un support : une table, une partition, un système de fichiers.
//!
//! Mêmes garde-fous que l'écriture : filtre, identité revérifiée, démontage, `O_EXCL`, `yes`, `force` pour
//! un disque interne. FAT32 est écrit ici et relu ; exFAT, NTFS et ext4 passent par l'hôte une fois la
//! table écrite et la partition apparue.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_core::BlockDevice;
use bonk_core::format::table::{self, PartitionPlan, TableKind};
use bonk_core::format::{Filesystem, FormatError, Ids, fat32, normalize_label};
use bonk_core::partition::{Severity, inspect};
use bonk_hal::platform::format_capability;
use bonk_hal::{Access, Elevation, FilterOptions};

use super::target::{Chosen, OpenRequest, choose, enumerate, open_target, reread_table};
use super::types::{Deps, FormatDone, JobError, SCHEMA_VERSION, Target, TraceIo, fail};
use super::verify::drop_cache_before_verify;
use super::write::confirm_internal;
use crate::errors::Code;

/// Au-delà, `exfat` plutôt que `fat32` : la limite usuelle des outils et le seuil attendu des appareils.
const FAT32_DEFAULT_LIMIT: u64 = 32 << 30;
/// Regards sur le système en attendant la partition ou le système de fichiers : une demi-minute avec la
/// pause de 250 ms.
const PARTITION_LOOKS: u32 = 120;

pub struct FormatJob {
    pub target: Target,
    /// `None` : [`default_filesystem`].
    pub filesystem: Option<Filesystem>,
    pub table: TableKind,
    pub label: Option<String>,
    pub options: FilterOptions,
    pub trace_io: Option<TraceIo>,
    pub dry_run: bool,
    pub yes: bool,
    pub force: bool,
    pub elevation: Elevation,
}

/// Système de fichiers choisi quand rien n'est demandé.
#[must_use]
pub fn default_filesystem(size: u64) -> Filesystem {
    if size <= FAT32_DEFAULT_LIMIT {
        Filesystem::Fat32
    } else {
        Filesystem::Exfat
    }
}

fn format_error(err: FormatError) -> JobError {
    fail(Code::FormatFailed, err)
}

/// Refuse avant tout geste un système de fichiers que cet hôte ne sait pas écrire sur cette cible.
fn check_available(fs: Filesystem, chosen: &Chosen, deps: &Deps<'_>) -> Result<(), JobError> {
    if fs.is_native() {
        return Ok(());
    }
    if chosen.disk.is_none() {
        return Err(fail(
            Code::FilesystemUnavailable,
            format!(
                "{fs} passe par les outils du système : pas sur un fichier image, seul fat32 l'est"
            ),
        ));
    }
    let name = format_capability(fs);
    let capability = (deps.io.capabilities)()
        .into_iter()
        .find(|capability| capability.name == name);
    match capability {
        Some(capability) if capability.available => Ok(()),
        Some(capability) => Err(fail(
            Code::FilesystemUnavailable,
            format!(
                "{fs} indisponible : {} ; --fs fat32 n'exige aucun outil",
                capability.detail.unwrap_or_default()
            ),
        )),
        None => Err(fail(
            Code::FilesystemUnavailable,
            format!("{fs} indisponible sur cet hôte"),
        )),
    }
}

/// Graine des identifiants du support : deux formatages ne doivent pas donner les mêmes GUID. Seule source
/// d'horloge et de hasard du formatage : `bonk-core` n'en a pas, ses tests restent déterministes.
pub(super) fn fresh_seed(target: &str) -> Vec<u8> {
    use std::hash::{BuildHasher, RandomState};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let random = RandomState::new().hash_one(target);
    format!("{target}:{nanos}:{}:{random}", std::process::id()).into_bytes()
}

/// La table vient d'être relue : pas d'erreur, et la partition voulue.
fn check_table(dev: &dyn BlockDevice, part: PartitionPlan) -> Result<(), JobError> {
    let layout = inspect(dev).map_err(|e| fail(Code::FormatFailed, e))?;
    if let Some(finding) = layout
        .findings
        .iter()
        .find(|finding| finding.severity == Severity::Error)
    {
        return Err(fail(
            Code::FormatFailed,
            format!("table relue incorrecte : {}", finding.message),
        ));
    }
    let in_gpt = layout.gpt.as_ref().is_some_and(|gpt| {
        gpt.entries
            .iter()
            .any(|e| e.first_lba == part.first_lba && e.last_lba == part.last_lba)
    });
    let in_mbr = layout.mbr.as_ref().is_some_and(|mbr| {
        mbr.entries
            .iter()
            .any(|e| u64::from(e.start_lba) == part.first_lba)
    });
    if in_gpt || in_mbr {
        Ok(())
    } else {
        Err(fail(
            Code::FormatFailed,
            "table relue incorrecte : partition absente",
        ))
    }
}

/// Attend que le système montre **la nouvelle** partition, reconnue à sa taille : il peut garder un moment
/// l'ancienne table, et formater une ancienne partition de la même place détruirait la mauvaise zone.
fn wait_for_partition(
    id: &str,
    expected_bytes: u64,
    deps: &Deps<'_>,
) -> Result<Option<String>, JobError> {
    for _ in 0..PARTITION_LOOKS {
        let disks = enumerate(deps)?;
        let found = disks
            .iter()
            .find(|disk| disk.id == id)
            .filter(|disk| disk.partitions.len() == 1)
            .map(|disk| &disk.partitions[0])
            .filter(|partition| partition.size_bytes == expected_bytes);
        if let Some(partition) = found {
            return Ok(Some(partition.path.clone()));
        }
        if (deps.io.cancelled)() {
            return Err(fail(Code::Cancelled, "annulée en attendant la partition"));
        }
        (deps.io.pause)();
    }
    Ok(None)
}

/// L'outil de l'hôte ne se relit pas ici octet par octet : sa sortie n'est pas interprétée. Le système
/// doit au moins reconnaître le résultat.
fn wait_for_filesystem(id: &str, fs: Filesystem, deps: &Deps<'_>) -> Result<(), JobError> {
    for _ in 0..PARTITION_LOOKS {
        let disks = enumerate(deps)?;
        let found = disks
            .iter()
            .find(|disk| disk.id == id)
            .and_then(|disk| disk.partitions.first())
            .and_then(|partition| partition.fs_type.as_deref());
        if found == Some(fs.name()) {
            return Ok(());
        }
        (deps.io.pause)();
    }
    Err(fail(
        Code::FormatFailed,
        format!("le système ne reconnaît pas {fs} sur la nouvelle partition"),
    ))
}

/// Écrit la table et, pour FAT32, le système de fichiers, puis relit ce qui a été écrit.
fn write_volume(
    device: &mut dyn BlockDevice,
    kind: TableKind,
    filesystem: Filesystem,
    label: Option<&str>,
    ids: &Ids,
) -> Result<(PartitionPlan, Option<u32>), JobError> {
    let partition =
        table::write_table(device, kind, filesystem, label, ids).map_err(format_error)?;
    let volume = fat32::Volume {
        first_lba: partition.first_lba,
        sectors: partition.sectors(),
        label,
        volume_id: ids.volume_id,
    };
    let cluster_bytes = if filesystem.is_native() {
        let geometry = fat32::write(device, &volume).map_err(format_error)?;
        Some(geometry.cluster_bytes)
    } else {
        None
    };
    device.flush().map_err(|e| fail(Code::FormatFailed, e))?;
    drop_cache_before_verify(device);
    check_table(device, partition)?;
    if filesystem.is_native() {
        fat32::verify(device, &volume).map_err(format_error)?;
    }
    Ok((partition, cluster_bytes))
}

/// Le disque formaté, tel que le système le désigne.
struct HostDisk<'a> {
    id: &'a str,
    path: &'a str,
    elevation: Elevation,
}

/// Le disque est rendu au système : il relit la table, montre la partition, et y pose exFAT, NTFS ou
/// ext4 avec ses propres outils.
fn hand_over_to_host(
    disk: &HostDisk<'_>,
    partition_bytes: u64,
    filesystem: Filesystem,
    label: Option<&str>,
    deps: &Deps<'_>,
) -> Result<(), JobError> {
    reread_table(Path::new(disk.path), disk.elevation, deps);
    let path = wait_for_partition(disk.id, partition_bytes, deps)?;
    if !filesystem.is_native() {
        let path = path.ok_or_else(|| {
            fail(
                Code::FormatFailed,
                "le système ne montre pas la nouvelle partition : rien n'a été formaté dessus",
            )
        })?;
        (deps.io.format_partition)(Path::new(&path), filesystem, label, disk.elevation)
            .map_err(|e| fail(Code::FormatFailed, e))?;
        wait_for_filesystem(disk.id, filesystem, deps)?;
    } else if path.is_none() {
        // FAT32 est écrit et relu : le système le verra au prochain branchement.
        log::warn!(target: "bonk::write", "new partition not visible yet: unplug and replug the disk");
    }
    Ok(())
}

pub fn format(job: &FormatJob, deps: &Deps<'_>) -> Result<FormatDone, JobError> {
    let chosen = choose(&job.target, job.options, deps)?;
    let size = chosen.info.size_bytes.ok_or_else(|| {
        fail(
            Code::InvalidArgument,
            "taille de l'image cible inconnue : la donner avec --size",
        )
    })?;
    let filesystem = job.filesystem.unwrap_or_else(|| default_filesystem(size));
    let label = job
        .label
        .as_deref()
        .map(|label| normalize_label(filesystem, label))
        .transpose()
        .map_err(|message| fail(Code::InvalidArgument, message))?;
    check_available(filesystem, &chosen, deps)?;

    // Refus par la géométrie avant tout geste. Le secteur réel n'est connu qu'à l'ouverture : 512 ici, le
    // plan est recalculé ensuite, et FAT32 se refuse de nouveau, alors que la table est déjà écrite.
    let block = bonk_core::DEFAULT_BLOCK_SIZE;
    let planned = table::plan(size, block, job.table).map_err(format_error)?;
    let cluster_bytes = if filesystem.is_native() {
        let geometry = fat32::geometry(planned.sectors(), block).map_err(format_error)?;
        Some(geometry.cluster_bytes)
    } else {
        None
    };
    let result = |event, dry_run, info, partition, cluster_bytes, unmounted, verified| FormatDone {
        schema_version: SCHEMA_VERSION,
        event,
        dry_run,
        target: info,
        table: job.table,
        filesystem,
        filesystem_defaulted: job.filesystem.is_none(),
        file_size_limit_bytes: filesystem.file_size_limit(),
        label: label.clone(),
        partition,
        written_by: if filesystem.is_native() {
            "bonk"
        } else {
            "system"
        },
        cluster_bytes,
        unmounted,
        verified,
    };
    if job.dry_run {
        let (info, mounts) = (chosen.info, chosen.mounts);
        return Ok(result(
            "plan",
            true,
            info,
            planned,
            cluster_bytes,
            mounts,
            false,
        ));
    }
    if !job.yes && !job.force {
        return Err(fail(
            Code::ConfirmationRequired,
            "formatage sans confirmation : ajouter --yes ou --force (ou décrire l'opération avec --dry-run)",
        ));
    }
    confirm_internal(&chosen, job.options, job.force)?;

    let seed_name = chosen.info.path.clone();
    let disk_id = chosen.disk.as_ref().map(|disk| disk.id.clone());
    let (mut device, info, unmounted) = open_target(
        chosen,
        &job.target,
        &OpenRequest {
            options: job.options,
            access: Access::ReadWrite,
            elevation: job.elevation,
            size_hint: Some(size),
            trace_io: job.trace_io,
        },
        deps,
    )?;
    if (deps.io.cancelled)() {
        return Err(fail(Code::Cancelled, "annulée avant le formatage"));
    }
    let ids = Ids::from_seed(&fresh_seed(&seed_name));
    log::info!(
        target: "bonk::write",
        target_path = info.path.as_str(), table = job.table.name(), filesystem = filesystem.name();
        "format starting"
    );
    let (partition, cluster_bytes) =
        write_volume(&mut *device, job.table, filesystem, label.as_deref(), &ids)?;
    let block = u64::from(device.block_size());
    drop(device);
    if let Some(id) = &disk_id {
        let disk = HostDisk {
            id,
            path: &info.path,
            elevation: job.elevation,
        };
        hand_over_to_host(
            &disk,
            partition.sectors() * block,
            filesystem,
            label.as_deref(),
            deps,
        )?;
    }
    log::info!(target: "bonk::write", filesystem = filesystem.name(); "format finished");
    Ok(result(
        "result",
        false,
        info,
        partition,
        cluster_bytes,
        unmounted,
        true,
    ))
}
