//! Faits bruts sur les disques de macOS et leur traduction en [`Disk`].
//!
//! La collecte (`platform/macos.rs`) lit `diskutil` ; cette traduction est pure et se teste, se rejoue et
//! se relit sur tous les systèmes.
//!
//! # APFS
//! Les volumes d'un disque APFS sont portés par un disque *synthétisé* (`disk3`) adossé à une partition
//! physique (`disk0s2`). Leurs montages sont rapportés sur cette partition : sans cela, le disque qui porte
//! le système paraîtrait libre.
//!
//! # Échouer plutôt que deviner
//! Sans volume système retrouvé sur un disque physique, l'énumération échoue.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::disk::{Bus, Disk, DiskKind, Partition};
use crate::enumerate::{EnumerateError, SourceRead};

/// Nom de la source qui porte ces faits dans une capture.
pub const SOURCE: &str = "macos:facts";

/// Ce que `diskutil info` dit d'un disque, d'une partition ou d'un volume, sans interprétation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RawMedia {
    /// `disk0`, `disk0s2`.
    pub device: String,
    pub whole: bool,
    /// Disque entier de rattachement.
    pub parent: String,
    pub bus: Option<String>,
    pub internal: bool,
    pub removable: bool,
    pub name: Option<String>,
    pub size_bytes: u64,
    /// Type de contenu : `EFI`, `Apple_APFS`, `Microsoft Basic Data`...
    pub content: Option<String>,
    pub file_system: Option<String>,
    pub label: Option<String>,
    pub mount: Option<String>,
    /// Pour un volume ou un disque APFS : les partitions physiques qui le portent.
    pub physical_stores: Vec<String>,
}

/// Lit la sortie de `diskutil info -plist`, convertie en JSON.
///
/// # Errors
/// `DeviceIdentifier` absent : la sortie n'a pas la forme attendue.
pub fn media_from_info(info: &Value) -> Result<RawMedia, EnumerateError> {
    let text = |key: &str| {
        info[key]
            .as_str()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let device = text("DeviceIdentifier").ok_or_else(|| EnumerateError::Malformed {
        what: "diskutil info".to_owned(),
        detail: "no DeviceIdentifier".to_owned(),
    })?;
    let flag = |key: &str| info[key].as_bool().unwrap_or(false);
    Ok(RawMedia {
        whole: flag("WholeDisk"),
        parent: text("ParentWholeDisk").unwrap_or_else(|| device.clone()),
        bus: text("BusProtocol"),
        internal: flag("Internal"),
        removable: flag("RemovableMediaOrExternalDevice") || flag("Removable"),
        name: text("MediaName").or_else(|| text("IORegistryEntryName")),
        size_bytes: info["TotalSize"]
            .as_u64()
            .or_else(|| info["Size"].as_u64())
            .unwrap_or(0),
        content: text("Content"),
        file_system: text("FilesystemType"),
        label: text("VolumeName"),
        mount: text("MountPoint"),
        physical_stores: info["APFSPhysicalStores"]
            .as_array()
            .map(|stores| {
                stores
                    .iter()
                    .filter_map(|s| s["APFSPhysicalStore"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default(),
        device,
    })
}

/// Valeurs de `BusProtocol`. Un bus inconnu reste interne par prudence, Thunderbolt compris : ce bus peut
/// aussi être celui d'un disque interne.
fn bus_of(media: &RawMedia) -> (Bus, DiskKind) {
    let bus = match media.bus.as_deref() {
        Some("USB") => Bus::Usb,
        Some("Secure Digital") => Bus::Sd,
        Some("SATA" | "ATA") => Bus::Sata,
        Some("PCI-Express" | "NVMe" | "Apple Fabric") => Bus::Nvme,
        Some("SAS" | "SCSI" | "Fibre Channel Interface") => Bus::Scsi,
        Some("Disk Image" | "Virtual Interface") => return (Bus::Virtual, DiskKind::HostVirtual),
        _ => Bus::Unknown,
    };
    (bus, DiskKind::Physical)
}

fn is_system_mount(mount: &str) -> bool {
    mount == "/" || mount.starts_with("/System/Volumes/")
}

fn stable_id(whole: &RawMedia, bus: Bus) -> String {
    let name = whole.name.as_deref().unwrap_or("disk");
    let bus = crate::describe::bus_label(bus, crate::describe::Lang::En);
    let id = format!("{bus}-{name}-{}", whole.size_bytes);
    id.split_whitespace().collect::<Vec<_>>().join("_")
}

/// Traduit les faits en disques.
///
/// # Errors
/// Aucun disque physique ne porte le système.
pub fn normalize(media: &[RawMedia]) -> Result<Vec<Disk>, EnumerateError> {
    // Les disques synthétisés ne sont pas écrivables : seuls leurs supports comptent.
    let synthesized = |m: &RawMedia| !m.physical_stores.is_empty();
    let mut disks: Vec<Disk> = media
        .iter()
        .filter(|m| m.whole && !synthesized(m))
        .map(|whole| {
            let (bus, kind) = bus_of(whole);
            let partitions: Vec<Partition> = media
                .iter()
                .filter(|m| !m.whole && m.parent == whole.device && !synthesized(m))
                .map(|part| {
                    let stacked: Vec<&RawMedia> = media
                        .iter()
                        .filter(|v| v.physical_stores.contains(&part.device))
                        .collect();
                    let mut mount_points: Vec<String> = part.mount.iter().cloned().collect();
                    mount_points.extend(stacked.iter().filter_map(|v| v.mount.clone()));
                    Partition {
                        path: format!("/dev/{}", part.device),
                        label: part.label.clone(),
                        fs_type: part.file_system.clone(),
                        size_bytes: part.size_bytes,
                        is_system_volume: mount_points.iter().any(|m| is_system_mount(m)),
                        mount_points,
                        holders: Vec::new(),
                        is_active_esp: false,
                    }
                })
                .collect();
            Disk {
                id: stable_id(whole, bus),
                path: format!("/dev/{}", whole.device),
                model: whole.name.clone().unwrap_or_default(),
                serial: None,
                size_bytes: whole.size_bytes,
                bus,
                kind,
                removable_flag: whole.removable,
                fs_type: None,
                holders: Vec::new(),
                partitions,
            }
        })
        .collect();
    if !disks
        .iter()
        .any(|d| d.partitions.iter().any(|p| p.is_system_volume))
    {
        return Err(EnumerateError::Malformed {
            what: SOURCE.to_owned(),
            detail: "no physical disk holds the system volume".to_owned(),
        });
    }
    // L'ESP du disque qui porte le système est celle dont l'hôte a démarré.
    for disk in &mut disks {
        if disk.partitions.iter().any(|p| p.is_system_volume) {
            let esps: Vec<String> = media
                .iter()
                .filter(|m| m.content.as_deref() == Some("EFI"))
                .map(|m| format!("/dev/{}", m.device))
                .collect();
            for part in &mut disk.partitions {
                part.is_active_esp = esps.contains(&part.path);
            }
        }
    }
    let ids: Vec<String> = disks.iter().map(|d| d.id.clone()).collect();
    for disk in &mut disks {
        if ids.iter().filter(|id| **id == disk.id).count() > 1 {
            disk.id = format!("{}-{}", disk.id, disk.path.trim_start_matches("/dev/"));
        }
    }
    Ok(disks)
}

/// Chemin brut d'un disque entier : `/dev/disk2` donne `/dev/rdisk2`. Ni partition, ni autre périphérique.
#[must_use]
pub fn raw_disk_path(path: &str) -> Option<String> {
    let digits = path.strip_prefix("/dev/disk")?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("/dev/rdisk{digits}"))
}

/// La lecture de la capture qui porte ces faits, s'il y en a une.
pub(crate) fn from_capture(reads: &[SourceRead]) -> Option<Result<Vec<Disk>, EnumerateError>> {
    let read = reads.iter().find(|read| read.path == SOURCE)?;
    let parsed: Result<Vec<RawMedia>, String> = read
        .content
        .as_deref()
        .ok_or_else(|| "no content".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()));
    Some(match parsed {
        Ok(media) => normalize(&media),
        Err(detail) => Err(EnumerateError::Malformed {
            what: SOURCE.to_owned(),
            detail,
        }),
    })
}

/// Le contenu d'une capture, sans ce qui identifie le propriétaire. Un contenu illisible disparaît.
#[must_use]
pub fn sanitize(content: &str) -> String {
    let Ok(mut media) = serde_json::from_str::<Vec<RawMedia>>(content) else {
        return String::new();
    };
    for item in &mut media {
        item.label = item.label.as_ref().map(|_| "<masked>".to_owned());
        if let Some(mount) = &item.mount
            && mount.starts_with("/Volumes/")
        {
            item.mount = Some("/Volumes/<masked>".to_owned());
        }
    }
    serde_json::to_string(&media).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};

    fn whole(device: &str, bus: &str) -> RawMedia {
        RawMedia {
            device: device.to_owned(),
            whole: true,
            parent: device.to_owned(),
            bus: Some(bus.to_owned()),
            name: Some("Media".to_owned()),
            size_bytes: 1 << 40,
            ..RawMedia::default()
        }
    }

    fn part(device: &str, parent: &str, content: &str) -> RawMedia {
        RawMedia {
            device: device.to_owned(),
            parent: parent.to_owned(),
            content: Some(content.to_owned()),
            size_bytes: 1 << 30,
            ..RawMedia::default()
        }
    }

    /// Mac : système sur APFS, ESP, clé USB montée dans `/Volumes`.
    fn mac() -> Vec<RawMedia> {
        let mut container = whole("disk3", "PCI-Express");
        container.physical_stores = vec!["disk0s2".to_owned()];
        let mut systeme = part("disk3s1", "disk3", "41504653");
        systeme.mount = Some("/".to_owned());
        systeme.physical_stores = vec!["disk0s2".to_owned()];
        let mut cle = part("disk2s1", "disk2", "Microsoft Basic Data");
        cle.mount = Some("/Volumes/CLE".to_owned());
        cle.file_system = Some("msdos".to_owned());
        vec![
            whole("disk0", "PCI-Express"),
            part("disk0s1", "disk0", "EFI"),
            part("disk0s2", "disk0", "Apple_APFS"),
            container,
            systeme,
            whole("disk2", "USB"),
            cle,
        ]
    }

    #[test]
    fn le_disque_qui_porte_apfs_est_interdit_et_la_cle_proposee() -> Result<(), EnumerateError> {
        let disks = normalize(&mac())?;
        // Le disque synthétisé n'est pas listé.
        assert_eq!(disks.len(), 2);
        assert_eq!(disks[0].path, "/dev/disk0");
        assert!(disks[0].partitions[1].is_system_volume);
        assert!(disks[0].partitions[0].is_active_esp);
        assert!(matches!(
            evaluate(&disks[0], FilterOptions::default()),
            Eligibility::Excluded { reasons }
                if reasons.iter().any(|r| matches!(r, Exclusion::SystemVolume { .. }))
        ));
        assert!(matches!(
            evaluate(&disks[1], FilterOptions::default()),
            Eligibility::Offered { .. }
        ));
        Ok(())
    }

    #[test]
    fn sans_systeme_retrouve_l_enumeration_echoue() {
        let sans: Vec<RawMedia> = mac()
            .into_iter()
            .filter(|m| m.device != "disk3s1")
            .collect();
        assert!(normalize(&sans).is_err());
        // Un montage ailleurs que sur `/` ne fait pas un système.
        let mut ailleurs = mac();
        ailleurs[4].mount = Some("/Volumes/x".to_owned());
        assert!(normalize(&ailleurs).is_err());
    }

    #[test]
    fn une_image_est_virtuelle_et_un_bus_inconnu_reste_interne() -> Result<(), EnumerateError> {
        let mut media = mac();
        media[5].bus = Some("Disk Image".to_owned());
        assert_eq!(normalize(&media)?[1].bus, Bus::Virtual);
        media[5].bus = Some("Thunderbolt".to_owned());
        assert_eq!(normalize(&media)?[1].bus, Bus::Unknown);
        Ok(())
    }

    #[test]
    fn seul_un_disque_entier_a_un_chemin_brut() {
        assert_eq!(raw_disk_path("/dev/disk2").as_deref(), Some("/dev/rdisk2"));
        for refuse in ["/dev/disk2s1", "/dev/disk", "/dev/rdisk2", "/tmp/disk2"] {
            assert_eq!(raw_disk_path(refuse), None, "{refuse}");
        }
    }

    #[test]
    fn lecture_de_diskutil_info() -> Result<(), EnumerateError> {
        let info = serde_json::json!({
            "DeviceIdentifier": "disk3s1",
            "ParentWholeDisk": "disk3",
            "WholeDisk": false,
            "BusProtocol": "Apple Fabric",
            "Internal": true,
            "TotalSize": 1024,
            "FilesystemType": "apfs",
            "VolumeName": " Macintosh HD ",
            "MountPoint": "",
            "APFSPhysicalStores": [{"APFSPhysicalStore": "disk0s2"}]
        });
        let media = media_from_info(&info)?;
        assert_eq!(media.parent, "disk3");
        assert_eq!(media.label.as_deref(), Some("Macintosh HD"));
        assert_eq!(media.mount, None);
        assert_eq!(media.size_bytes, 1024);
        assert_eq!(media.physical_stores, ["disk0s2"]);
        assert!(media_from_info(&serde_json::json!({})).is_err());
        Ok(())
    }

    #[test]
    fn doublons_distingues_et_capture_masquee() -> Result<(), Box<dyn std::error::Error>> {
        let mut media = mac();
        media.push(whole("disk5", "USB"));
        let disks = normalize(&media)?;
        assert_eq!(disks[1].id, "usb-Media-1099511627776-disk2");
        assert_eq!(disks[2].id, "usb-Media-1099511627776-disk5");

        let propre = sanitize(&serde_json::to_string(&mac())?);
        assert!(!propre.contains("/Volumes/CLE"), "{propre}");
        assert_eq!(sanitize("illisible"), "");
        let read = SourceRead {
            path: SOURCE.to_owned(),
            kind: "file".to_owned(),
            ok: true,
            bytes: None,
            error: None,
            micros: 0,
            content: Some(propre),
        };
        assert_eq!(
            from_capture(std::slice::from_ref(&read))
                .ok_or("capture")??
                .len(),
            2
        );
        Ok(())
    }
}
