//! Énumération des disques sous Linux, en lecture seule.
//!
//! Sources, toutes lisibles sans privilège :
//! - `/sys/block` : disques, partitions, tailles, périphériques empilés (`holders`), et le chemin
//!   canonique du périphérique, d'où l'on déduit le bus (USB, SATA, NVMe, carte SD...) ;
//! - `/proc/self/mountinfo` et `/proc/swaps` : ce que l'hôte utilise en ce moment ;
//! - la base udev (`/run/udev/data`) : signatures de systèmes de fichiers, numéro de série, type de
//!   partition (ESP) et liens `by-id`, que seul udev sait lire sans ouvrir le périphérique.
//!
//! Pourquoi pas UDisks2 (D-Bus) : il demanderait un service en marche et une dépendance de plus pour
//! des informations déjà disponibles en lecture de fichiers. Un disque que l'appelant n'a pas le droit
//! d'ouvrir passe, lui, par UDisks2 et polkit (`platform/udisks.rs`).
//!
//! Tous les chemins système sont regroupés dans [`LinuxRoots`] : les tests construisent une
//! arborescence factice et vérifient l'énumération sans toucher au vrai système.

mod mounts;
mod replay;
mod sources;
#[cfg(test)]
mod tests;
mod udev;

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::disk::{Bus, Disk, DiskKind, Partition};
use crate::enumerate::EnumerateError;

use mounts::{MountIndex, active_swaps, parse_mountinfo};
use sources::{
    canonical, list_dir, list_dir_optional, note_link, read_devnum, read_optional, read_required,
    read_size,
};
use udev::{UdevRecord, load_udev};

pub use replay::enumerate_from_reads;
pub use sources::enumerate_recorded;

/// Type de partition GPT « EFI System Partition ».
const GPT_ESP: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

/// Type de partition MBR « EFI System ».
const MBR_ESP: &str = "0xef";

/// Points de montage où l'ESP active se trouve en pratique.
const ESP_MOUNTS: [&str; 3] = ["/boot/efi", "/efi", "/boot"];

/// Emplacements des sources système.
#[derive(Debug, Clone)]
pub struct LinuxRoots {
    pub sys: PathBuf,
    pub proc: PathBuf,
    pub udev_data: PathBuf,
    pub dev: PathBuf,
}

/// Énumère les disques à partir de `roots`.
pub fn enumerate(roots: &LinuxRoots) -> Result<Vec<Disk>, EnumerateError> {
    let mountinfo_path = roots.proc.join("self/mountinfo");
    let mounts = parse_mountinfo(&read_required(&mountinfo_path)?)?;
    let swaps = active_swaps(roots)?;
    if !roots.udev_data.is_dir() {
        // Sans base udev, aucune signature RAID, LVM, swap ou ZFS n'est connue : ne rien proposer.
        return Err(EnumerateError::Malformed {
            what: roots.udev_data.display().to_string(),
            detail: "udev database not found: disk signatures cannot be checked".to_owned(),
        });
    }

    let sys_block = roots.sys.join("block");
    let canonical_sys = canonical(&roots.sys)?;
    let ctx = Context {
        roots,
        canonical_sys,
        mounts: MountIndex::new(roots, &mounts),
        swaps,
    };

    let started = Instant::now();
    let mut names = list_dir(&sys_block)?;
    names.sort_by(|a, b| natural_cmp(a, b));
    log::debug!(
        target: "bonk_hal::linux",
        candidates = names.len(), mounts = mounts.len(), swaps = ctx.swaps.len();
        "enumerating block devices"
    );
    let mut disks = Vec::new();
    for name in names {
        match read_disk(&ctx, &sys_block.join(&name), &name)? {
            Some(disk) => {
                log::debug!(
                    target: "bonk_hal::linux",
                    id = disk.id.as_str(), path = disk.path.as_str(), bus = format!("{:?}", disk.bus).as_str(),
                    kind = format!("{:?}", disk.kind).as_str(), size_bytes = disk.size_bytes,
                    partitions = disk.partitions.len();
                    "disk found"
                );
                disks.push(disk);
            }
            None => {
                log::trace!(target: "bonk_hal::linux", device = name.as_str(); "device ignored (stacked, memory, optical or hidden)")
            }
        }
    }
    log::debug!(
        target: "bonk_hal::linux",
        disks = disks.len(), millis = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        "enumeration done"
    );
    Ok(disks)
}

struct Context<'a> {
    roots: &'a LinuxRoots,
    canonical_sys: PathBuf,
    mounts: MountIndex,
    /// Noms des périphériques utilisés comme espace d'échange en ce moment.
    swaps: BTreeSet<String>,
}

fn read_disk(ctx: &Context<'_>, dir: &Path, name: &str) -> Result<Option<Disk>, EnumerateError> {
    // Périphériques empilés (device-mapper, RAID logiciel), disques mémoire et lecteurs optiques :
    // ce ne sont pas des cibles, ils apparaissent comme `holders` des vrais disques.
    if is_ignored(name) || read_optional(&dir.join("hidden")).as_deref() == Some("1") {
        return Ok(None);
    }
    let real = canonical(dir)?;
    let relative = real.strip_prefix(&ctx.canonical_sys).unwrap_or(&real);
    note_link(dir, relative);
    let components: Vec<&str> = relative
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();
    let (kind, bus) = classify(&components, name, dir);

    let devnum = read_devnum(dir)?;
    let udev = load_udev(ctx.roots, &devnum)?;
    let size_bytes = read_size(dir)?;
    let fs_type = signature(&udev, ctx.swaps.contains(name));
    let path = format!("/dev/{name}");

    let mut partitions = read_partitions(ctx, dir)?;
    let disk_mounts = ctx.mounts.of(&devnum, name);
    if partitions.is_empty() && !disk_mounts.is_empty() {
        // Système de fichiers posé sur le disque entier, sans table de partitions : il faut quand
        // même voir son montage.
        partitions.push(Partition {
            path: path.clone(),
            label: udev.prop("ID_FS_LABEL").map(str::to_owned),
            fs_type: None,
            size_bytes,
            is_system_volume: disk_mounts.iter().any(|m| m == "/"),
            is_active_esp: is_active_esp(&udev, &disk_mounts),
            mount_points: disk_mounts,
            holders: Vec::new(),
        });
    }

    Ok(Some(Disk {
        id: stable_id(&udev, name),
        path,
        model: model(dir, &udev, name),
        serial: udev
            .prop("ID_SERIAL_SHORT")
            .map(str::to_owned)
            .or_else(|| read_optional(&dir.join("device/serial"))),
        size_bytes,
        bus,
        kind,
        removable_flag: read_optional(&dir.join("removable")).as_deref() == Some("1"),
        fs_type,
        holders: list_dir_optional(&dir.join("holders"))?,
        partitions,
    }))
}

fn read_partitions(ctx: &Context<'_>, disk_dir: &Path) -> Result<Vec<Partition>, EnumerateError> {
    let mut found: Vec<(u64, Partition)> = Vec::new();
    for entry in list_dir(disk_dir)? {
        let dir = disk_dir.join(&entry);
        let Some(number) = read_optional(&dir.join("partition")) else {
            continue;
        };
        let number: u64 = number.parse().map_err(|_| EnumerateError::Malformed {
            what: dir.join("partition").display().to_string(),
            detail: format!("unreadable partition number: {number:?}"),
        })?;
        let devnum = read_devnum(&dir)?;
        let udev = load_udev(ctx.roots, &devnum)?;
        let mounts = ctx.mounts.of(&devnum, &entry);
        found.push((
            number,
            Partition {
                path: format!("/dev/{entry}"),
                label: udev.prop("ID_FS_LABEL").map(str::to_owned),
                fs_type: signature(&udev, ctx.swaps.contains(entry.as_str())),
                size_bytes: read_size(&dir)?,
                is_system_volume: mounts.iter().any(|m| m == "/"),
                is_active_esp: is_active_esp(&udev, &mounts),
                holders: list_dir_optional(&dir.join("holders"))?,
                mount_points: mounts,
            },
        ));
    }
    found.sort_by_key(|(number, _)| *number);
    Ok(found.into_iter().map(|(_, partition)| partition).collect())
}

fn is_ignored(name: &str) -> bool {
    let numbered = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit() || c == '_'))
    };
    name.starts_with("dm-") || numbered("md") || numbered("ram") || numbered("sr") || numbered("fd")
}

/// Nature et bus d'un disque, déduits du chemin canonique de son périphérique sous `/sys`.
fn classify(components: &[&str], name: &str, dir: &Path) -> (DiskKind, Bus) {
    if components.starts_with(&["devices", "virtual"]) {
        let kind = if name.starts_with("loop") {
            DiskKind::Loop
        } else if name.starts_with("zram") {
            DiskKind::Zram
        } else {
            DiskKind::HostVirtual
        };
        return (kind, Bus::Virtual);
    }
    let numbered = |c: &str, prefix: &str| {
        c.strip_prefix(prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|d| d.is_ascii_digit()))
    };
    let bus = if components.iter().any(|c| numbered(c, "usb")) {
        Bus::Usb
    } else if components.contains(&"mmc_host") {
        // Une carte SD est amovible ; une mémoire eMMC soldée ne l'est pas.
        match read_optional(&dir.join("device/type")).as_deref() {
            Some("SD") => Bus::Sd,
            _ => Bus::Unknown,
        }
    } else if components.contains(&"nvme") {
        Bus::Nvme
    } else if components.iter().any(|c| numbered(c, "ata")) {
        Bus::Sata
    } else if components
        .iter()
        .any(|c| c.starts_with("virtio") || c.starts_with("vmbus") || c.starts_with("xen"))
    {
        Bus::Virtual
    } else if components.iter().any(|c| numbered(c, "host")) {
        Bus::Scsi
    } else {
        Bus::Unknown
    };
    (DiskKind::Physical, bus)
}

fn model(dir: &Path, udev: &UdevRecord, name: &str) -> String {
    let vendor = read_optional(&dir.join("device/vendor"));
    let model = read_optional(&dir.join("device/model"))
        .or_else(|| read_optional(&dir.join("device/name")))
        .or_else(|| udev.prop("ID_MODEL").map(|m| m.replace('_', " ")));
    let joined = match (vendor, model) {
        (Some(v), Some(m)) if !m.starts_with(&v) => format!("{v} {m}"),
        (_, Some(m)) => m,
        (Some(v), None) => v,
        (None, None) => String::new(),
    };
    // Les attributs sysfs sont complétés d'espaces à largeur fixe.
    let collapsed = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        name.to_owned()
    } else {
        collapsed
    }
}

/// Identifiant stable : le lien `by-id` le plus lisible, jamais `/dev/sdX`. À défaut (loop,
/// zram, certains lecteurs de carte), `dev-<nom>` : stable seulement le temps du branchement.
fn stable_id(udev: &UdevRecord, name: &str) -> String {
    let mut candidates: Vec<&str> = udev
        .symlinks
        .iter()
        .filter_map(|s| s.strip_prefix("disk/by-id/"))
        .collect();
    // Les identifiants WWN et EUI sont stables mais illisibles : seulement en dernier recours.
    let readable =
        |s: &&str| !(s.starts_with("wwn-") || s.contains("eui.") || s.starts_with("nvme-uuid."));
    candidates.sort_by_key(|s| (!readable(s), s.len(), s.to_owned()));
    candidates
        .first()
        .map_or_else(|| format!("dev-{name}"), |s| (*s).to_owned())
}

fn signature(udev: &UdevRecord, swapped: bool) -> Option<String> {
    udev.prop("ID_FS_TYPE")
        .map(str::to_owned)
        .or_else(|| swapped.then(|| "swap".to_owned()))
}

fn is_active_esp(udev: &UdevRecord, mounts: &[String]) -> bool {
    let is_esp = udev
        .prop("ID_PART_ENTRY_TYPE")
        .is_some_and(|t| t.eq_ignore_ascii_case(GPT_ESP) || t.eq_ignore_ascii_case(MBR_ESP));
    is_esp && mounts.iter().any(|m| ESP_MOUNTS.contains(&m.as_str()))
}

/// Ordre « naturel » : `loop2` avant `loop10`.
fn natural_cmp(a: &str, b: &str) -> Ordering {
    fn chunks(s: &str) -> Vec<(bool, &str)> {
        let mut out = Vec::new();
        let mut start = 0;
        let mut prev: Option<bool> = None;
        for (i, c) in s.char_indices() {
            let digit = c.is_ascii_digit();
            if prev.is_some_and(|p| p != digit) {
                out.push((prev == Some(true), &s[start..i]));
                start = i;
            }
            prev = Some(digit);
        }
        if prev.is_some() {
            out.push((prev == Some(true), &s[start..]));
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(&cb) {
        let order = if x.0 && y.0 {
            let (nx, ny) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
            nx.len().cmp(&ny.len()).then_with(|| nx.cmp(ny))
        } else {
            x.1.cmp(y.1)
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    ca.len().cmp(&cb.len())
}

impl LinuxRoots {
    /// Le vrai système.
    #[must_use]
    pub fn system() -> Self {
        Self {
            sys: "/sys".into(),
            proc: "/proc".into(),
            udev_data: "/run/udev/data".into(),
            dev: "/dev".into(),
        }
    }

    /// Une arborescence factice, pour les tests.
    #[must_use]
    pub fn under(base: &Path) -> Self {
        Self {
            sys: base.join("sys"),
            proc: base.join("proc"),
            udev_data: base.join("run/udev/data"),
            dev: base.join("dev"),
        }
    }
}
