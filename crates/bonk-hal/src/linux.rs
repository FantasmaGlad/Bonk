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
//! des informations déjà disponibles en lecture de fichiers. Les accès aux disques eux-mêmes (lot
//! ultérieur) passeront bien par UDisks2 et polkit (D-03).
//!
//! Tous les chemins système sont regroupés dans [`LinuxRoots`] : les tests construisent une
//! arborescence factice et vérifient l'énumération sans toucher au vrai système.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::disk::{Bus, Disk, DiskKind, Partition};
use crate::enumerate::{EnumerateError, SourceRead};

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

/// Énumère les disques à partir de `roots`.
///
/// # Errors
/// [`EnumerateError`] si les points de montage, `/proc/swaps`, la base udev ou la liste des disques
/// sont illisibles : voir la documentation du module `enumerate`.
pub fn enumerate(roots: &LinuxRoots) -> Result<Vec<Disk>, EnumerateError> {
    let mountinfo_path = roots.proc.join("self/mountinfo");
    let mounts = parse_mountinfo(&read_required(&mountinfo_path)?)?;
    let swaps = active_swaps(roots)?;
    if !roots.udev_data.is_dir() {
        // Sans base udev, aucune signature RAID, LVM, swap ou ZFS n'est connue : ne rien proposer.
        return Err(EnumerateError::Malformed {
            what: roots.udev_data.display().to_string(),
            detail:
                "base udev introuvable : les signatures des disques ne peuvent pas être vérifiées"
                    .to_owned(),
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
            detail: format!("numéro de partition illisible : {number:?}"),
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

/// Identifiant stable : le lien `by-id` le plus lisible, jamais `/dev/sdX` (CDC §3.3). À défaut (loop,
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

// ---------------------------------------------------------------------------------------------
// Points de montage
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct MountEntry {
    devnum: String,
    mount_point: String,
    source: String,
}

/// Lit `/proc/self/mountinfo`. Une ligne illisible fait échouer l'énumération : mieux vaut ne rien
/// proposer qu'ignorer un montage.
fn parse_mountinfo(text: &str) -> Result<Vec<MountEntry>, EnumerateError> {
    let malformed = |line: &str| EnumerateError::Malformed {
        what: "mountinfo".to_owned(),
        detail: format!("ligne illisible : {line:?}"),
    };
    let mut entries = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        // Un point de montage contenant « - » est échappé, donc le premier « - » isolé est bien le
        // séparateur entre les champs fixes et les champs de type de système de fichiers.
        let (before, after) = line.split_once(" - ").ok_or_else(|| malformed(line))?;
        let fields: Vec<&str> = before.split_whitespace().collect();
        let tail: Vec<&str> = after.split_whitespace().collect();
        if fields.len() < 5 || tail.len() < 2 {
            return Err(malformed(line));
        }
        entries.push(MountEntry {
            devnum: fields[2].to_owned(),
            mount_point: unescape_octal(fields[4]),
            source: unescape_octal(tail[1]),
        });
    }
    Ok(entries)
}

/// Défait l'échappement du noyau : `\040` pour l'espace, `\011` la tabulation, `\134` l'antislash...
fn unescape_octal(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = (bytes[i] == b'\\' && i + 4 <= bytes.len())
            .then(|| &bytes[i + 1..i + 4])
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)))
            .map(|d| d.iter().fold(0u32, |acc, b| acc * 8 + u32::from(b - b'0')))
            .and_then(|v| u8::try_from(v).ok());
        if let Some(byte) = octal {
            out.push(byte);
            i += 4;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Montages indexés par numéro de périphérique **et** par nom.
///
/// Le numéro seul ne suffit pas : btrfs rapporte un numéro anonyme (`0:50`) et seule la source
/// (`/dev/sdc1`) désigne le vrai périphérique.
struct MountIndex {
    by_devnum: BTreeMap<String, Vec<String>>,
    by_name: BTreeMap<String, Vec<String>>,
}

impl MountIndex {
    fn new(roots: &LinuxRoots, mounts: &[MountEntry]) -> Self {
        let mut by_devnum: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for mount in mounts {
            by_devnum
                .entry(mount.devnum.clone())
                .or_default()
                .push(mount.mount_point.clone());
            if let Some(name) = device_name(roots, &mount.source) {
                by_name
                    .entry(name)
                    .or_default()
                    .push(mount.mount_point.clone());
            }
        }
        Self { by_devnum, by_name }
    }

    fn of(&self, devnum: &str, name: &str) -> Vec<String> {
        let mut all: Vec<String> = self
            .by_devnum
            .get(devnum)
            .into_iter()
            .chain(self.by_name.get(name))
            .flatten()
            .cloned()
            .collect();
        all.sort();
        all.dedup();
        all
    }
}

/// Nom du périphérique (`sda1`) derrière une source de montage (`/dev/sda1`, `/dev/disk/by-uuid/...`).
fn device_name(roots: &LinuxRoots, source: &str) -> Option<String> {
    let relative = source.strip_prefix("/dev/")?;
    let path = roots.dev.join(relative);
    let resolved = fs::canonicalize(&path).unwrap_or(path);
    resolved.file_name()?.to_str().map(str::to_owned)
}

fn active_swaps(roots: &LinuxRoots) -> Result<BTreeSet<String>, EnumerateError> {
    let text = read_required(&roots.proc.join("swaps"))?;
    Ok(text
        .lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().next())
        .filter_map(|source| device_name(roots, &unescape_octal(source)))
        .collect())
}

// ---------------------------------------------------------------------------------------------
// Base udev
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
struct UdevRecord {
    symlinks: Vec<String>,
    props: BTreeMap<String, String>,
}

impl UdevRecord {
    fn prop(&self, key: &str) -> Option<&str> {
        self.props
            .get(key)
            .map(String::as_str)
            .filter(|v| !v.is_empty())
    }
}

fn parse_udev(text: &str) -> UdevRecord {
    let mut record = UdevRecord::default();
    for line in text.lines() {
        if let Some(link) = line.strip_prefix("S:") {
            record.symlinks.push(link.to_owned());
        } else if let Some((key, value)) = line.strip_prefix("E:").and_then(|e| e.split_once('=')) {
            record.props.insert(key.to_owned(), value.to_owned());
        }
    }
    record
}

/// Enregistrement udev d'un périphérique ; vide s'il n'en a pas (le répertoire existe : voir
/// [`enumerate`]).
fn load_udev(roots: &LinuxRoots, devnum: &str) -> Result<UdevRecord, EnumerateError> {
    let path = roots.udev_data.join(format!("b{devnum}"));
    match traced_read(&path) {
        Ok(text) => Ok(parse_udev(&text)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(UdevRecord::default()),
        Err(err) => Err(io_error(&path, err)),
    }
}

// ---------------------------------------------------------------------------------------------
// Lecture de fichiers
// ---------------------------------------------------------------------------------------------

// ---------------------------------------------------------------------------------------------
// Trace des lectures : journal au niveau `trace` et enregistrement pour `bonk debug sources`
// ---------------------------------------------------------------------------------------------

/// Taille maximale du contenu conservé par lecture enregistrée.
const CONTENT_LIMIT: usize = 8 * 1024;

thread_local! {
    // Enregistrement propre au fil d'exécution qui énumère : la fenêtre énumère sur un autre fil et
    // n'est donc jamais gênée par un `debug sources` en cours.
    static RECORDER: RefCell<Option<Vec<SourceRead>>> = const { RefCell::new(None) };
}

fn note_read(
    kind: &'static str,
    path: &Path,
    started: Instant,
    outcome: Result<(usize, &str), &io::Error>,
) {
    let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    let shown = path.display().to_string();
    match &outcome {
        Ok((bytes, _)) => log::trace!(
            target: "bonk_hal::linux",
            path = shown.as_str(), kind = kind, bytes = *bytes, micros = micros;
            "read source"
        ),
        Err(err) => log::trace!(
            target: "bonk_hal::linux",
            path = shown.as_str(), kind = kind, error = err.to_string().as_str(), micros = micros;
            "read source failed"
        ),
    }
    RECORDER.with(|recorder| {
        if let Some(reads) = recorder.borrow_mut().as_mut() {
            let (bytes, error, content) = match outcome {
                Ok((bytes, content)) => {
                    let mut kept = content.to_owned();
                    if kept.len() > CONTENT_LIMIT {
                        let mut end = CONTENT_LIMIT;
                        while !kept.is_char_boundary(end) {
                            end -= 1;
                        }
                        kept.truncate(end);
                    }
                    (Some(bytes), None, Some(kept))
                }
                Err(err) => (None, Some(err.to_string()), None),
            };
            reads.push(SourceRead {
                path: shown,
                kind,
                ok: error.is_none(),
                bytes,
                error,
                micros,
                content,
            });
        }
    });
}

fn traced_read(path: &Path) -> io::Result<String> {
    let started = Instant::now();
    let result = fs::read_to_string(path);
    note_read(
        "file",
        path,
        started,
        result.as_ref().map(|text| (text.len(), text.as_str())),
    );
    result
}

fn traced_list(path: &Path) -> io::Result<Vec<String>> {
    let started = Instant::now();
    let result = (|| {
        let mut names = Vec::new();
        for entry in fs::read_dir(path)? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    })();
    let joined = result.as_ref().map(|names: &Vec<String>| names.join("\n"));
    note_read(
        "dir",
        path,
        started,
        joined
            .as_ref()
            .map_err(|erreur| *erreur)
            .map(|text| (text.len(), text.as_str())),
    );
    result
}

/// Énumère comme [`enumerate`] et rend aussi toutes les lectures de sources, pour `bonk debug sources`.
pub fn enumerate_recorded(
    roots: &LinuxRoots,
) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
    RECORDER.with(|recorder| *recorder.borrow_mut() = Some(Vec::new()));
    let result = enumerate(roots);
    let reads = RECORDER.with(|recorder| recorder.borrow_mut().take().unwrap_or_default());
    (result, reads)
}

fn io_error(path: &Path, source: io::Error) -> EnumerateError {
    EnumerateError::Io {
        what: path.display().to_string(),
        source,
    }
}

fn read_required(path: &Path) -> Result<String, EnumerateError> {
    traced_read(path).map_err(|e| io_error(path, e))
}

/// Attribut descriptif : absent ou illisible vaut « inconnu ».
fn read_optional(path: &Path) -> Option<String> {
    let text = traced_read(path).ok()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn canonical(path: &Path) -> Result<PathBuf, EnumerateError> {
    fs::canonicalize(path).map_err(|e| io_error(path, e))
}

fn list_dir(path: &Path) -> Result<Vec<String>, EnumerateError> {
    traced_list(path).map_err(|e| io_error(path, e))
}

/// Comme [`list_dir`], mais un répertoire absent est vide (`holders` n'existe que pour les disques
/// que le noyau a rendus visibles).
fn list_dir_optional(path: &Path) -> Result<Vec<String>, EnumerateError> {
    match traced_list(path) {
        Ok(mut names) => {
            names.sort();
            Ok(names)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(io_error(path, err)),
    }
}

fn read_devnum(dir: &Path) -> Result<String, EnumerateError> {
    Ok(read_required(&dir.join("dev"))?.trim().to_owned())
}

fn read_size(dir: &Path) -> Result<u64, EnumerateError> {
    let path = dir.join("size");
    let text = read_required(&path)?;
    let sectors: u64 = text.trim().parse().map_err(|_| EnumerateError::Malformed {
        what: path.display().to_string(),
        detail: format!("taille illisible : {:?}", text.trim()),
    })?;
    // Le noyau exprime toujours `size` en secteurs de 512 octets, quelle que soit la taille logique.
    sectors
        .checked_mul(512)
        .ok_or_else(|| EnumerateError::Malformed {
            what: path.display().to_string(),
            detail: "taille hors limites".to_owned(),
        })
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

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

    use super::*;
    use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};

    /// Arborescence factice supprimée à la fin du test.
    struct Fake {
        base: PathBuf,
    }

    impl Fake {
        fn new() -> Result<Self, Box<dyn Error>> {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let n = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
            let base = std::env::temp_dir().join(format!("bonk-enum-{}-{n}", std::process::id()));
            let fake = Self { base };
            fake.put("proc/self/mountinfo", "")?;
            fake.put(
                "proc/swaps",
                "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n",
            )?;
            fs::create_dir_all(fake.base.join("run/udev/data"))?;
            fs::create_dir_all(fake.base.join("sys/block"))?;
            fs::create_dir_all(fake.base.join("dev"))?;
            Ok(fake)
        }

        fn roots(&self) -> LinuxRoots {
            LinuxRoots::under(&self.base)
        }

        fn put(&self, rel: &str, content: &str) -> Result<(), Box<dyn Error>> {
            let path = self.base.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, content)?;
            Ok(())
        }

        /// Crée un disque sous `sys/<device_path>` et le lien `sys/block/<name>`.
        fn disk(
            &self,
            name: &str,
            device_path: &str,
            devnum: &str,
            sectors: u64,
            extra: &[(&str, &str)],
        ) -> Result<(), Box<dyn Error>> {
            let dir = format!("sys/{device_path}");
            self.put(&format!("{dir}/dev"), &format!("{devnum}\n"))?;
            self.put(&format!("{dir}/size"), &format!("{sectors}\n"))?;
            self.put(&format!("{dir}/removable"), "0\n")?;
            fs::create_dir_all(self.base.join(&dir).join("holders"))?;
            for (file, content) in extra {
                self.put(&format!("{dir}/{file}"), content)?;
            }
            symlink(
                format!("../{device_path}"),
                self.base.join("sys/block").join(name),
            )?;
            Ok(())
        }

        fn partition(
            &self,
            device_path: &str,
            name: &str,
            number: u32,
            devnum: &str,
            sectors: u64,
        ) -> Result<(), Box<dyn Error>> {
            let dir = format!("sys/{device_path}/{name}");
            self.put(&format!("{dir}/dev"), &format!("{devnum}\n"))?;
            self.put(&format!("{dir}/size"), &format!("{sectors}\n"))?;
            self.put(&format!("{dir}/partition"), &format!("{number}\n"))?;
            fs::create_dir_all(self.base.join(&dir).join("holders"))?;
            Ok(())
        }

        fn udev(&self, devnum: &str, lines: &str) -> Result<(), Box<dyn Error>> {
            self.put(&format!("run/udev/data/b{devnum}"), lines)
        }

        fn mountinfo(&self, text: &str) -> Result<(), Box<dyn Error>> {
            self.put("proc/self/mountinfo", text)
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.base);
        }
    }

    const NVME_PATH: &str = "devices/pci0000:00/0000:00:02.4/0000:02:00.0/nvme/nvme0/nvme0n1";
    const USB_PATH: &str =
        "devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb";
    const SATA_PATH: &str =
        "devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda";

    /// Un poste dont le système est sur NVMe, d'après la structure relevée sur une machine réelle.
    fn desktop(fake: &Fake) -> Result<(), Box<dyn Error>> {
        fake.disk(
            "nvme0n1",
            NVME_PATH,
            "259:0",
            1_000_215_216,
            &[("device/model", "SAMSUNG MZVL2512HDJD-00BH1       \n")],
        )?;
        fake.partition(NVME_PATH, "nvme0n1p1", 1, "259:1", 2_097_152)?;
        fake.partition(NVME_PATH, "nvme0n1p2", 2, "259:2", 997_000_000)?;
        fake.udev(
            "259:0",
            "S:disk/by-id/nvme-eui.002538bc31bf297e\nS:disk/by-id/nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4\nS:disk/by-id/nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4_1\nE:ID_SERIAL_SHORT=S6Z4\nE:ID_MODEL=SAMSUNG MZVL2512HDJD-00BH1\n",
        )?;
        fake.udev(
            "259:1",
            "E:ID_FS_TYPE=vfat\nE:ID_PART_ENTRY_TYPE=c12a7328-f81f-11d2-ba4b-00a0c93ec93b\n",
        )?;
        fake.udev("259:2", "E:ID_FS_TYPE=ext4\n")?;
        fake.mountinfo(
            "44 1 259:2 / / rw,noatime shared:1 - ext4 /dev/nvme0n1p2 rw,commit=60\n\
             64 44 259:1 / /boot/efi rw,relatime shared:134 - vfat /dev/nvme0n1p1 rw,fmask=0022\n\
             34 44 0:29 / /run rw,nosuid,nodev shared:13 - tmpfs tmpfs rw,size=3017756k\n",
        )
    }

    fn usb_stick(fake: &Fake) -> Result<(), Box<dyn Error>> {
        fake.disk(
            "sdb",
            USB_PATH,
            "8:16",
            31_260_672,
            &[
                ("removable", "1\n"),
                ("device/vendor", "SanDisk \n"),
                ("device/model", "Cruzer Blade    \n"),
            ],
        )?;
        fake.partition(USB_PATH, "sdb1", 1, "8:17", 31_258_624)?;
        fake.udev(
            "8:16",
            "S:disk/by-id/usb-SanDisk_Cruzer_Blade_4C530001-0:0\nE:ID_SERIAL_SHORT=4C530001\nE:ID_BUS=usb\n",
        )?;
        fake.udev("8:17", "E:ID_FS_TYPE=vfat\nE:ID_FS_LABEL=CLE\n")
    }

    fn verdict(disk: &Disk) -> Eligibility {
        evaluate(disk, FilterOptions::default())
    }

    fn find<'a>(disks: &'a [Disk], path: &str) -> Result<&'a Disk, Box<dyn Error>> {
        disks
            .iter()
            .find(|d| d.path == path)
            .ok_or_else(|| format!("disque {path} absent de {disks:?}").into())
    }

    #[test]
    fn disque_systeme_nvme_est_exclu() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        let disks = enumerate(&fake.roots())?;
        assert_eq!(disks.len(), 1);
        let nvme = &disks[0];
        assert_eq!(nvme.bus, Bus::Nvme);
        assert_eq!(nvme.kind, DiskKind::Physical);
        assert_eq!(nvme.size_bytes, 1_000_215_216 * 512);
        assert_eq!(nvme.model, "SAMSUNG MZVL2512HDJD-00BH1");
        assert_eq!(nvme.serial.as_deref(), Some("S6Z4"));
        // Le lien lisible l'emporte sur l'identifiant EUI ; le plus court sur sa variante « _1 ».
        assert_eq!(nvme.id, "nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4");
        let [esp, root] = nvme.partitions.as_slice() else {
            return Err(format!("deux partitions attendues : {:?}", nvme.partitions).into());
        };
        assert!(esp.is_active_esp && !esp.is_system_volume);
        assert_eq!(esp.mount_points, ["/boot/efi"]);
        assert!(root.is_system_volume && !root.is_active_esp);
        let Eligibility::Excluded { reasons } = verdict(nvme) else {
            return Err("le disque système doit être exclu".into());
        };
        assert!(reasons.contains(&Exclusion::SystemVolume {
            partition: "/dev/nvme0n1p2".to_owned()
        }));
        assert!(reasons.contains(&Exclusion::ActiveEsp {
            partition: "/dev/nvme0n1p1".to_owned()
        }));
        Ok(())
    }

    #[test]
    fn cle_usb_montee_par_l_utilisateur_est_proposee() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        usb_stick(&fake)?;
        let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
        info.push_str("90 44 8:17 / /run/media/alice/CLE rw,nosuid shared:2 - vfat /dev/sdb1 rw\n");
        fake.mountinfo(&info)?;
        let disks = enumerate(&fake.roots())?;
        let usb = find(&disks, "/dev/sdb")?;
        assert_eq!(usb.bus, Bus::Usb);
        assert!(usb.removable_flag);
        assert_eq!(usb.model, "SanDisk Cruzer Blade");
        assert_eq!(usb.id, "usb-SanDisk_Cruzer_Blade_4C530001-0:0");
        assert_eq!(usb.partitions[0].label.as_deref(), Some("CLE"));
        assert_eq!(usb.partitions[0].mount_points, ["/run/media/alice/CLE"]);
        assert!(verdict(usb).is_writable());
        Ok(())
    }

    #[test]
    fn cle_usb_non_montee_est_proposee() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        usb_stick(&fake)?;
        let disks = enumerate(&fake.roots())?;
        assert!(verdict(find(&disks, "/dev/sdb")?).is_writable());
        Ok(())
    }

    #[test]
    fn systeme_de_fichiers_sur_le_disque_entier_reste_visible() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        fake.disk(
            "sdc",
            &USB_PATH.replace("sdb", "sdc"),
            "8:32",
            2_048_000,
            &[],
        )?;
        fake.udev("8:32", "E:ID_FS_TYPE=ext4\n")?;
        let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
        info.push_str("91 44 8:32 / /mnt/cle rw - ext4 /dev/sdc rw\n");
        fake.mountinfo(&info)?;
        let disks = enumerate(&fake.roots())?;
        let sdc = find(&disks, "/dev/sdc")?;
        assert!(sdc.partitions.len() == 1 && sdc.partitions[0].path == "/dev/sdc");
        assert_eq!(sdc.partitions[0].mount_points, ["/mnt/cle"]);
        assert!(!verdict(sdc).is_writable());
        Ok(())
    }

    #[test]
    fn montage_btrfs_retrouve_par_la_source() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        usb_stick(&fake)?;
        // btrfs rapporte un numéro anonyme (0:50) : seule la source désigne le périphérique, ici
        // par un lien comme en crée udev.
        fs::create_dir_all(fake.base.join("dev/disk/by-uuid"))?;
        fs::write(fake.base.join("dev/sdb1"), "")?;
        symlink("../../sdb1", fake.base.join("dev/disk/by-uuid/abcd"))?;
        let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
        info.push_str(
            "92 44 0:50 /sous-volume /mnt/donnees rw - btrfs /dev/disk/by-uuid/abcd rw\n",
        );
        fake.mountinfo(&info)?;
        let disks = enumerate(&fake.roots())?;
        let usb = find(&disks, "/dev/sdb")?;
        assert_eq!(usb.partitions[0].mount_points, ["/mnt/donnees"]);
        assert!(!verdict(usb).is_writable());
        Ok(())
    }

    #[test]
    fn point_de_montage_avec_espace() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        usb_stick(&fake)?;
        let mut info = fs::read_to_string(fake.base.join("proc/self/mountinfo"))?;
        info.push_str("90 44 8:17 / /run/media/alice/MA\\040CLE rw - vfat /dev/sdb1 rw\n");
        fake.mountinfo(&info)?;
        let disks = enumerate(&fake.roots())?;
        assert_eq!(
            find(&disks, "/dev/sdb")?.partitions[0].mount_points,
            ["/run/media/alice/MA CLE"]
        );
        Ok(())
    }

    #[test]
    fn volume_chiffre_ouvert_est_empile() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        usb_stick(&fake)?;
        fake.udev("8:17", "E:ID_FS_TYPE=crypto_LUKS\n")?;
        fs::write(
            fake.base
                .join("sys")
                .join(USB_PATH)
                .join("sdb1/holders/dm-0"),
            "",
        )?;
        let disks = enumerate(&fake.roots())?;
        let usb = find(&disks, "/dev/sdb")?;
        assert_eq!(usb.partitions[0].holders, ["dm-0"]);
        assert!(!verdict(usb).is_writable());
        Ok(())
    }

    #[test]
    fn swap_actif_detecte_meme_sans_signature_udev() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        usb_stick(&fake)?;
        fake.udev("8:17", "")?;
        fs::write(fake.base.join("dev/sdb1"), "")?;
        fake.put(
            "proc/swaps",
            "Filename\tType\tSize\tUsed\tPriority\n/dev/sdb1\tpartition\t1024\t0\t-2\n",
        )?;
        let disks = enumerate(&fake.roots())?;
        let usb = find(&disks, "/dev/sdb")?;
        assert_eq!(usb.partitions[0].fs_type.as_deref(), Some("swap"));
        assert!(!verdict(usb).is_writable());
        Ok(())
    }

    #[test]
    fn disque_sata_et_carte_sd() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        fake.disk("sda", SATA_PATH, "8:0", 1_953_525_168, &[])?;
        let sd = "devices/pci0000:00/0000:00:1a.0/mmc_host/mmc0/mmc0:0001/block/mmcblk0";
        fake.disk(
            "mmcblk0",
            sd,
            "179:0",
            31_116_288,
            &[("device/type", "SD\n")],
        )?;
        let emmc = "devices/platform/mmc_host/mmc1/mmc1:0001/block/mmcblk1";
        fake.disk(
            "mmcblk1",
            emmc,
            "179:8",
            15_269_888,
            &[("device/type", "MMC\n")],
        )?;
        let disks = enumerate(&fake.roots())?;
        assert_eq!(find(&disks, "/dev/sda")?.bus, Bus::Sata);
        assert_eq!(find(&disks, "/dev/mmcblk0")?.bus, Bus::Sd);
        assert_eq!(find(&disks, "/dev/mmcblk1")?.bus, Bus::Unknown);
        assert!(verdict(find(&disks, "/dev/mmcblk0")?).is_writable());
        assert!(!verdict(find(&disks, "/dev/mmcblk1")?).is_writable());
        Ok(())
    }

    #[test]
    fn disques_virtuels_et_empiles() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        for (name, devnum) in [("loop0", "7:0"), ("zram0", "251:0"), ("nbd0", "43:0")] {
            fake.disk(
                name,
                &format!("devices/virtual/block/{name}"),
                devnum,
                2048,
                &[],
            )?;
        }
        for (name, devnum) in [
            ("dm-0", "253:0"),
            ("md127", "9:127"),
            ("sr0", "11:0"),
            ("ram0", "1:0"),
        ] {
            fake.disk(
                name,
                &format!("devices/virtual/block/{name}"),
                devnum,
                2048,
                &[],
            )?;
        }
        let disks = enumerate(&fake.roots())?;
        let names: Vec<&str> = disks.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            names,
            ["/dev/loop0", "/dev/nbd0", "/dev/nvme0n1", "/dev/zram0"]
        );
        assert_eq!(find(&disks, "/dev/loop0")?.kind, DiskKind::Loop);
        assert_eq!(find(&disks, "/dev/zram0")?.kind, DiskKind::Zram);
        assert_eq!(find(&disks, "/dev/nbd0")?.kind, DiskKind::HostVirtual);
        assert_eq!(find(&disks, "/dev/loop0")?.bus, Bus::Virtual);
        assert_eq!(find(&disks, "/dev/loop0")?.id, "dev-loop0");
        Ok(())
    }

    #[test]
    fn nvme_cache_est_ignore() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        fake.disk(
            "nvme0c0n1",
            "devices/pci0000:00/nvme/nvme0/nvme0c0n1",
            "259:9",
            2048,
            &[("hidden", "1\n")],
        )?;
        assert_eq!(enumerate(&fake.roots())?.len(), 1);
        Ok(())
    }

    #[test]
    fn tri_naturel_des_disques() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        for (i, name) in ["loop10", "loop2", "loop1"].iter().enumerate() {
            fake.disk(
                name,
                &format!("devices/virtual/block/{name}"),
                &format!("7:{i}"),
                8,
                &[],
            )?;
        }
        let disks = enumerate(&fake.roots())?;
        let names: Vec<&str> = disks.iter().map(|d| d.path.as_str()).collect();
        assert_eq!(
            names,
            ["/dev/loop1", "/dev/loop2", "/dev/loop10", "/dev/nvme0n1"]
        );
        Ok(())
    }

    #[test]
    fn base_udev_absente_fait_echouer() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        fs::remove_dir_all(fake.base.join("run/udev/data"))?;
        assert!(matches!(
            enumerate(&fake.roots()),
            Err(EnumerateError::Malformed { .. })
        ));
        Ok(())
    }

    #[test]
    fn montages_illisibles_font_echouer() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        fs::remove_file(fake.base.join("proc/self/mountinfo"))?;
        assert!(matches!(
            enumerate(&fake.roots()),
            Err(EnumerateError::Io { .. })
        ));
        fake.mountinfo("ligne sans separateur\n")?;
        assert!(matches!(
            enumerate(&fake.roots()),
            Err(EnumerateError::Malformed { .. })
        ));
        Ok(())
    }

    #[test]
    fn taille_illisible_fait_echouer() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        fake.put(&format!("sys/{NVME_PATH}/size"), "beaucoup\n")?;
        assert!(matches!(
            enumerate(&fake.roots()),
            Err(EnumerateError::Malformed { .. })
        ));
        Ok(())
    }

    #[test]
    fn les_lectures_de_sources_sont_enregistrees_avec_leur_contenu() -> Result<(), Box<dyn Error>> {
        let fake = Fake::new()?;
        desktop(&fake)?;
        let (disks, reads) = enumerate_recorded(&fake.roots());
        assert_eq!(disks?.len(), 1);
        let mountinfo = reads
            .iter()
            .find(|r| r.path.ends_with("proc/self/mountinfo"))
            .ok_or("mountinfo non enregistré")?;
        assert!(mountinfo.ok && mountinfo.kind == "file");
        assert!(
            mountinfo
                .content
                .as_deref()
                .is_some_and(|c| c.contains("/boot/efi"))
        );
        // Les échecs sont enregistrés aussi : un fichier optionnel absent est une lecture ratée.
        assert!(
            reads.iter().any(|r| !r.ok && r.error.is_some()),
            "{reads:?}"
        );
        assert!(
            reads
                .iter()
                .any(|r| r.kind == "dir" && r.path.ends_with("sys/block"))
        );
        // L'enregistrement s'arrête avec l'appel : une énumération ordinaire ne garde rien.
        let again = enumerate(&fake.roots())?;
        assert_eq!(again.len(), 1);
        let (_, none_left) = enumerate_recorded(&fake.roots());
        assert_eq!(
            none_left
                .iter()
                .filter(|r| r.path.ends_with("proc/swaps"))
                .count(),
            1
        );
        Ok(())
    }

    #[test]
    fn echappements_octaux() {
        assert_eq!(unescape_octal("/a\\040b"), "/a b");
        assert_eq!(unescape_octal("/a\\134b"), "/a\\b");
        assert_eq!(unescape_octal("/a\\04"), "/a\\04");
        assert_eq!(unescape_octal("/a\\"), "/a\\");
        assert_eq!(unescape_octal("/a\\999"), "/a\\999");
        assert_eq!(unescape_octal("/sans/echappement"), "/sans/echappement");
    }

    #[test]
    fn ordre_naturel() {
        let mut names = vec!["loop10", "loop2", "sda", "nvme0n1", "loop1", "sdb", "sdaa"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            ["loop1", "loop2", "loop10", "nvme0n1", "sda", "sdaa", "sdb"]
        );
    }
}
