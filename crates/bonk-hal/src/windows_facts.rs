//! Faits bruts sur les disques de Windows et leur traduction en [`Disk`].
//!
//! La collecte (`platform/windows.rs`) n'existe que sous Windows ; cette traduction est pure et se teste,
//! se rejoue et se relit sur tous les systèmes : une capture de Windows sert ainsi de jeu de test partout.
//!
//! # Échouer plutôt que deviner
//! Sans volume système retrouvé sur un disque, l'énumération échoue : le disque système paraîtrait libre.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::disk::{Bus, Disk, DiskKind, Partition};
use crate::enumerate::{EnumerateError, SourceRead};

/// Nom de la source qui porte ces faits dans une capture.
pub const SOURCE: &str = "windows:facts";

/// Type GPT de la partition système EFI.
const GPT_ESP: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
/// Types GPT des disques dynamiques (métadonnées et données).
const GPT_LDM: [&str; 2] = [
    "5808c8aa-7e8f-42e0-85d2-e1e90434cfb3",
    "af9b60a0-1431-4f62-bc68-3311714a69ad",
];
/// Type GPT d'un membre d'espace de stockage.
const GPT_SPACES: &str = "e75caf8f-f680-4cee-afa3-b001e56efc2d";
/// Types MBR de la partition système EFI et d'un disque dynamique.
const MBR_ESP: u8 = 0xEF;
const MBR_LDM: u8 = 0x42;

/// Ce que Windows dit de ses disques et de ses volumes, sans interprétation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WindowsFacts {
    /// `%SystemDrive%`, par exemple `C:`.
    pub system_drive: Option<String>,
    pub disks: Vec<RawDisk>,
    pub volumes: Vec<RawVolume>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RawDisk {
    /// `N` de `\\.\PhysicalDriveN`.
    pub number: u32,
    /// Valeur de `STORAGE_BUS_TYPE`.
    pub bus_type: i32,
    pub removable: bool,
    pub vendor: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub size_bytes: u64,
    /// GUID du disque (GPT) ou signature (MBR) : survit à un rebranchement, pas le numéro.
    pub disk_id: Option<String>,
    pub partitions: Vec<RawPartition>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RawPartition {
    pub number: u32,
    pub offset: u64,
    pub size_bytes: u64,
    pub gpt_type: Option<String>,
    pub mbr_type: Option<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RawVolume {
    /// `\\?\Volume{...}\`.
    pub name: String,
    pub mount_points: Vec<String>,
    /// Nom du système de fichiers selon Windows (`NTFS`, `FAT32`...).
    pub file_system: Option<String>,
    pub label: Option<String>,
    pub extents: Vec<RawExtent>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RawExtent {
    pub disk: u32,
    pub offset: u64,
}

/// Valeurs de `STORAGE_BUS_TYPE`. Un bus inconnu, ou un espace de stockage, reste interne par prudence.
fn bus_of(bus_type: i32) -> (Bus, DiskKind) {
    let bus = match bus_type {
        7 => Bus::Usb,
        12 | 13 => Bus::Sd,
        2 | 3 | 11 => Bus::Sata,
        17 => Bus::Nvme,
        1 | 6 | 8 | 9 | 10 => Bus::Scsi,
        14 | 15 => return (Bus::Virtual, DiskKind::HostVirtual),
        _ => Bus::Unknown,
    };
    (bus, DiskKind::Physical)
}

fn signature_of(partition: &RawPartition) -> Option<&'static str> {
    let gpt = partition.gpt_type.as_deref();
    if gpt.is_some_and(|t| GPT_LDM.contains(&t)) || partition.mbr_type == Some(MBR_LDM) {
        return Some("ldm_member");
    }
    (gpt == Some(GPT_SPACES)).then_some("storage_spaces_member")
}

fn is_esp(partition: &RawPartition) -> bool {
    partition.gpt_type.as_deref() == Some(GPT_ESP) || partition.mbr_type == Some(MBR_ESP)
}

/// Le nom de Windows ramené à celui du filtre, commun aux systèmes.
fn file_system_name(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "fat" | "fat32" => "vfat".to_owned(),
        other => other.to_owned(),
    }
}

fn is_on_drive(mount: &str, drive: &str) -> bool {
    mount
        .get(..drive.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(drive))
        && matches!(&mount[drive.len()..], "" | "\\" | "/")
}

fn stable_id(disk: &RawDisk, bus: Bus) -> String {
    let base = match (&disk.serial, &disk.disk_id) {
        (Some(serial), _) => format!(
            "{}-{serial}",
            crate::describe::bus_label(bus, crate::describe::Lang::En)
        ),
        (None, Some(id)) => format!("disk-{id}"),
        (None, None) => format!("physicaldrive{}", disk.number),
    };
    base.split_whitespace().collect::<Vec<_>>().join("_")
}

/// Traduit les faits en disques.
///
/// # Errors
/// `%SystemDrive%` inconnu, ou porté par aucun volume d'un disque énuméré.
pub fn normalize(facts: &WindowsFacts) -> Result<Vec<Disk>, EnumerateError> {
    let malformed = |detail: &str| EnumerateError::Malformed {
        what: SOURCE.to_owned(),
        detail: detail.to_owned(),
    };
    let drive = facts
        .system_drive
        .as_deref()
        .filter(|d| !d.is_empty())
        .ok_or_else(|| malformed("system drive unknown"))?;
    let is_system = |volume: &RawVolume| volume.mount_points.iter().any(|m| is_on_drive(m, drive));
    if !facts.volumes.iter().filter(|v| is_system(v)).any(|v| {
        v.extents
            .iter()
            .any(|e| facts.disks.iter().any(|d| d.number == e.disk))
    }) {
        return Err(malformed("no enumerated disk holds the system volume"));
    }

    let mut disks: Vec<Disk> = facts
        .disks
        .iter()
        .map(|raw| {
            let (bus, kind) = bus_of(raw.bus_type);
            let hosts_system = facts
                .volumes
                .iter()
                .any(|v| is_system(v) && v.extents.iter().any(|e| e.disk == raw.number));
            let path = format!(r"\\.\PhysicalDrive{}", raw.number);
            // Un volume réparti sur plusieurs morceaux, ou qui ne commence sur aucune partition connue,
            // est empilé sur le disque : ni la partition ni le disque ne sont libres.
            let holders = facts
                .volumes
                .iter()
                .filter(|v| {
                    v.extents.iter().any(|e| e.disk == raw.number)
                        && (v.extents.len() > 1
                            || !v.extents.iter().any(|e| {
                                raw.partitions.iter().any(|p| p.offset == e.offset)
                            }))
                })
                .map(|v| v.name.clone())
                .collect();
            let partitions = raw
                .partitions
                .iter()
                .map(|partition| {
                    let volume = facts.volumes.iter().find(|v| {
                        matches!(v.extents.as_slice(), [e] if e.disk == raw.number && e.offset == partition.offset)
                    });
                    let file_system = volume
                        .and_then(|v| v.file_system.as_deref())
                        .map(file_system_name);
                    Partition {
                        path: format!(r"{path}\Partition{}", partition.number),
                        label: volume.and_then(|v| v.label.clone()),
                        fs_type: signature_of(partition).map(str::to_owned).or(file_system),
                        size_bytes: partition.size_bytes,
                        mount_points: volume.map(|v| v.mount_points.clone()).unwrap_or_default(),
                        holders: Vec::new(),
                        is_system_volume: volume.is_some_and(is_system),
                        is_active_esp: hosts_system && is_esp(partition),
                    }
                })
                .collect();
            let model = [raw.vendor.as_deref(), raw.product.as_deref()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ");
            Disk {
                id: stable_id(raw, bus),
                path,
                model,
                serial: raw.serial.clone(),
                size_bytes: raw.size_bytes,
                bus,
                kind,
                removable_flag: raw.removable,
                fs_type: None,
                holders,
                partitions,
            }
        })
        .collect();
    disambiguate(&mut disks);
    Ok(disks)
}

/// Certains ponts USB donnent le même numéro de série à tous leurs exemplaires : un identifiant ambigu
/// pourrait désigner le mauvais disque.
fn disambiguate(disks: &mut [Disk]) {
    let ids: Vec<String> = disks.iter().map(|d| d.id.clone()).collect();
    for disk in disks.iter_mut() {
        if ids.iter().filter(|id| **id == disk.id).count() > 1 {
            let number = disk.path.rsplit("PhysicalDrive").next().unwrap_or_default();
            disk.id = format!("{}-{number}", disk.id);
        }
    }
}

/// Numéro `N` d'un chemin `\\\\.\\PhysicalDriveN` exact : ni partition, ni autre périphérique.
#[must_use]
pub fn disk_number(path: &str) -> Option<u32> {
    let digits = path.strip_prefix(r"\\.\PhysicalDrive")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// La lecture de la capture qui porte ces faits, s'il y en a une.
pub(crate) fn from_capture(reads: &[SourceRead]) -> Option<Result<Vec<Disk>, EnumerateError>> {
    let read = reads.iter().find(|read| read.path == SOURCE)?;
    let parsed: Result<WindowsFacts, String> = read
        .content
        .as_deref()
        .ok_or_else(|| "no content".to_owned())
        .and_then(|text| serde_json::from_str(text).map_err(|e| e.to_string()));
    Some(match parsed {
        Ok(facts) => normalize(&facts),
        Err(detail) => Err(EnumerateError::Malformed {
            what: SOURCE.to_owned(),
            detail,
        }),
    })
}

/// Le contenu d'une capture, sans ce qui identifie un exemplaire de matériel ou son propriétaire. Un
/// contenu illisible disparaît : on ne sait pas ce qu'il contient.
#[must_use]
pub fn sanitize(content: &str) -> String {
    let Ok(mut facts) = serde_json::from_str::<WindowsFacts>(content) else {
        return String::new();
    };
    for disk in &mut facts.disks {
        disk.serial = disk.serial.as_ref().map(|_| "<masked>".to_owned());
        disk.disk_id = disk.disk_id.as_ref().map(|_| "<masked>".to_owned());
    }
    for volume in &mut facts.volumes {
        volume.label = volume.label.as_ref().map(|_| "<masked>".to_owned());
    }
    serde_json::to_string(&facts).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};

    fn volume(name: &str, mount: &[&str], fs: &str, extents: &[(u32, u64)]) -> RawVolume {
        RawVolume {
            name: name.to_owned(),
            mount_points: mount.iter().map(|m| (*m).to_owned()).collect(),
            file_system: Some(fs.to_owned()),
            label: None,
            extents: extents
                .iter()
                .map(|&(disk, offset)| RawExtent { disk, offset })
                .collect(),
        }
    }

    fn part(number: u32, offset: u64, gpt: Option<&str>) -> RawPartition {
        RawPartition {
            number,
            offset,
            size_bytes: 1 << 30,
            gpt_type: gpt.map(str::to_owned),
            mbr_type: None,
        }
    }

    /// Disque système SATA avec ESP, clé USB montée en `E:`.
    fn poste() -> WindowsFacts {
        WindowsFacts {
            system_drive: Some("C:".to_owned()),
            disks: vec![
                RawDisk {
                    number: 0,
                    bus_type: 11,
                    product: Some("SSD".to_owned()),
                    serial: Some("S0".to_owned()),
                    size_bytes: 500 << 30,
                    partitions: vec![part(1, 1 << 20, Some(GPT_ESP)), part(2, 1 << 30, None)],
                    ..RawDisk::default()
                },
                RawDisk {
                    number: 1,
                    bus_type: 7,
                    vendor: Some("General".to_owned()),
                    product: Some("UDisk".to_owned()),
                    serial: Some("U 1".to_owned()),
                    size_bytes: 32 << 30,
                    partitions: vec![part(1, 1 << 20, None)],
                    ..RawDisk::default()
                },
            ],
            volumes: vec![
                volume("sys", &["c:\\"], "NTFS", &[(0, 1 << 30)]),
                volume("cle", &["E:\\"], "FAT32", &[(1, 1 << 20)]),
            ],
        }
    }

    fn verdict(disk: &Disk) -> Eligibility {
        evaluate(disk, FilterOptions::default())
    }

    #[test]
    fn seul_un_disque_entier_a_un_numero() {
        assert_eq!(disk_number(r"\\.\PhysicalDrive12"), Some(12));
        for refuse in [
            r"\\.\PhysicalDrive",
            r"\\.\PhysicalDrive1\Partition1",
            r"\\.\C:",
            "C:\\image.img",
            "PhysicalDrive1",
        ] {
            assert_eq!(disk_number(refuse), None, "{refuse}");
        }
    }

    #[test]
    fn le_disque_systeme_est_interdit_et_la_cle_proposee() -> Result<(), EnumerateError> {
        let disks = normalize(&poste())?;
        assert_eq!(disks[0].path, r"\\.\PhysicalDrive0");
        assert!(disks[0].partitions[1].is_system_volume);
        assert!(disks[0].partitions[0].is_active_esp);
        assert!(matches!(
            verdict(&disks[0]),
            Eligibility::Excluded { reasons }
                if reasons.iter().any(|r| matches!(r, Exclusion::SystemVolume { .. }))
        ));
        assert_eq!(disks[1].id, "usb-U_1");
        assert_eq!(disks[1].model, "General UDisk");
        assert_eq!(disks[1].partitions[0].fs_type.as_deref(), Some("vfat"));
        assert!(matches!(verdict(&disks[1]), Eligibility::Offered { .. }));
        Ok(())
    }

    #[test]
    fn un_disque_dur_usb_signale_fixe_reste_usb() -> Result<(), EnumerateError> {
        let mut facts = poste();
        facts.disks[1].removable = false;
        assert!(matches!(
            verdict(&normalize(&facts)?[1]),
            Eligibility::Offered { .. }
        ));
        Ok(())
    }

    #[test]
    fn sans_volume_systeme_retrouve_l_enumeration_echoue() {
        let mut inconnu = poste();
        inconnu.system_drive = None;
        assert!(normalize(&inconnu).is_err());
        let mut absent = poste();
        absent.system_drive = Some("Z:".to_owned());
        assert!(normalize(&absent).is_err());
        // Un dossier monté sous `C:` n'est pas le volume système.
        let mut sous_dossier = poste();
        sous_dossier.volumes[0].mount_points = vec!["C:\\mnt\\x\\".to_owned()];
        assert!(normalize(&sous_dossier).is_err());
    }

    #[test]
    fn un_volume_reparti_interdit_tous_ses_disques() -> Result<(), EnumerateError> {
        let mut facts = poste();
        facts
            .volumes
            .push(volume("span", &["F:\\"], "NTFS", &[(1, 1 << 20), (0, 9)]));
        let disks = normalize(&facts)?;
        assert_eq!(disks[1].holders, ["span"]);
        assert!(matches!(verdict(&disks[1]), Eligibility::Excluded { .. }));
        Ok(())
    }

    #[test]
    fn disques_dynamiques_et_espaces_de_stockage_sont_interdits() -> Result<(), EnumerateError> {
        for gpt in [GPT_LDM[0], GPT_LDM[1], GPT_SPACES] {
            let mut facts = poste();
            facts.disks[1].partitions[0].gpt_type = Some(gpt.to_owned());
            assert!(
                matches!(
                    verdict(&normalize(&facts)?[1]),
                    Eligibility::Excluded { .. }
                ),
                "{gpt}"
            );
        }
        let mut facts = poste();
        facts.disks[1].partitions[0].mbr_type = Some(MBR_LDM);
        assert!(matches!(
            verdict(&normalize(&facts)?[1]),
            Eligibility::Excluded { .. }
        ));
        Ok(())
    }

    #[test]
    fn un_vhd_est_virtuel_et_un_bus_inconnu_reste_interne() -> Result<(), EnumerateError> {
        let mut facts = poste();
        facts.disks[1].bus_type = 15;
        let disks = normalize(&facts)?;
        assert_eq!(
            (disks[1].bus, disks[1].kind),
            (Bus::Virtual, DiskKind::HostVirtual)
        );
        facts.disks[1].bus_type = 99;
        assert_eq!(normalize(&facts)?[1].bus, Bus::Unknown);
        Ok(())
    }

    #[test]
    fn identifiants_sans_serie_et_doublons() -> Result<(), EnumerateError> {
        let mut facts = poste();
        facts.disks[1].serial = None;
        facts.disks[1].disk_id = Some("abcd".to_owned());
        assert_eq!(normalize(&facts)?[1].id, "disk-abcd");
        facts.disks[1].disk_id = None;
        assert_eq!(normalize(&facts)?[1].id, "physicaldrive1");
        // Deux ponts USB au même numéro de série.
        let mut clone = facts.disks[1].clone();
        facts.disks[1].serial = Some("X".to_owned());
        clone.serial = Some("X".to_owned());
        clone.number = 2;
        facts.disks.push(clone);
        let disks = normalize(&facts)?;
        assert_eq!(
            (disks[1].id.as_str(), disks[2].id.as_str()),
            ("usb-X-1", "usb-X-2")
        );
        Ok(())
    }

    #[test]
    fn la_capture_se_rejoue_et_masque_ce_qui_identifie() -> Result<(), Box<dyn std::error::Error>> {
        let mut facts = poste();
        facts.volumes[1].label = Some("PERSONNEL".to_owned());
        let propre = sanitize(&serde_json::to_string(&facts)?);
        assert!(
            !propre.contains("S0") && !propre.contains("PERSONNEL"),
            "{propre}"
        );
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
        let disks = from_capture(std::slice::from_ref(&read)).ok_or("capture")??;
        assert_eq!(disks.len(), 2);
        assert!(from_capture(&[]).is_none());
        Ok(())
    }
}
