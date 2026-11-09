use std::fmt;

use crate::disk::Disk;
use crate::safety::{Eligibility, FilterOptions, evaluate};

#[derive(Debug, Clone)]
pub struct TargetIdentity {
    pub id: String,
    pub serial: Option<String>,
    pub size_bytes: u64,
    pub location: Option<String>,
}

impl From<&Disk> for TargetIdentity {
    fn from(disk: &Disk) -> Self {
        Self {
            id: disk.id.clone(),
            serial: disk.serial.clone(),
            size_bytes: disk.size_bytes,
            location: disk.location.clone(),
        }
    }
}

/// Without a serial number, two drives of the same model and size only differ by where they are plugged.
impl PartialEq for TargetIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.serial == other.serial
            && self.size_bytes == other.size_bytes
            && (self.serial.is_some() || self.location == other.location)
    }
}

impl Eq for TargetIdentity {}

#[derive(Debug)]
pub enum OpenTargetError {
    NotFound(String),
    Refused(Eligibility),
    IdentityChanged {
        expected: Box<TargetIdentity>,
        actual: Box<TargetIdentity>,
    },
    Unmount(String),
}

impl fmt::Display for OpenTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "disk not found: '{id}'"),
            Self::Refused(verdict) => write!(
                f,
                "{}",
                crate::describe::state_label(verdict, crate::describe::Lang::En)
            ),
            Self::IdentityChanged { expected, actual } => write!(
                f,
                "the disk changed: expected {} (serial {:?}, {} bytes), found {} (serial {:?}, {} bytes)",
                expected.id,
                expected.serial,
                expected.size_bytes,
                actual.id,
                actual.serial,
                actual.size_bytes
            ),
            Self::Unmount(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for OpenTargetError {}

/// Finds the target by stable id in a fresh enumeration, runs the filter and checks its identity, touching
/// nothing: called at choice time and again right before opening. Only an offered verdict allows the target.
pub fn reverify<'a>(
    expected: &TargetIdentity,
    disks: &'a [Disk],
    options: FilterOptions,
) -> Result<&'a Disk, OpenTargetError> {
    let disk = disks
        .iter()
        .find(|disk| disk.id == expected.id)
        .ok_or_else(|| OpenTargetError::NotFound(expected.id.clone()))?;
    let verdict = evaluate(disk, options);
    if !verdict.is_writable() {
        return Err(OpenTargetError::Refused(verdict));
    }
    let actual = TargetIdentity::from(disk);
    if actual != *expected {
        return Err(OpenTargetError::IdentityChanged {
            expected: Box::new(expected.clone()),
            actual: Box::new(actual),
        });
    }
    Ok(disk)
}

#[must_use]
pub fn mount_points(disk: &Disk) -> Vec<String> {
    let mut mounts: Vec<String> = disk
        .partitions
        .iter()
        .flat_map(|partition| partition.mount_points.iter().cloned())
        .collect();
    // The value breaks ties of equal length: `dedup` only removes neighbours.
    mounts.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    mounts.dedup();
    mounts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disk::{Bus, DiskKind, Partition};

    fn part(path: &str, mounts: &[&str], system: bool) -> Partition {
        Partition {
            path: path.to_owned(),
            label: None,
            fs_type: Some("vfat".to_owned()),
            size_bytes: 1 << 30,
            mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
            holders: vec![],
            is_system_volume: system,
            is_active_esp: false,
        }
    }

    fn disk(id: &str, serial: &str, size: u64, bus: Bus, partitions: Vec<Partition>) -> Disk {
        Disk {
            id: id.to_owned(),
            path: "/dev/sdb".to_owned(),
            model: "Cle".to_owned(),
            serial: Some(serial.to_owned()),
            size_bytes: size,
            bus,
            kind: DiskKind::Physical,
            removable_flag: true,
            logical_block_size: None,
            location: None,
            fs_type: None,
            holders: vec![],
            partitions,
        }
    }

    fn usb() -> Disk {
        disk(
            "usb-a",
            "S1",
            1 << 33,
            Bus::Usb,
            vec![part("/dev/sdb1", &["/run/media/u/A"], false)],
        )
    }

    /// Without a serial, the same model on another port is another target; with a serial, the port does not
    /// count.
    #[test]
    fn sans_serie_l_emplacement_departage_deux_cles_identiques() {
        let mut first = usb();
        first.serial = None;
        first.location = Some("pci-0000:05:00.3-usb-0:1:1.0".to_owned());
        let mut moved = first.clone();
        moved.location = Some("pci-0000:05:00.3-usb-0:2:1.0".to_owned());
        let expected = TargetIdentity::from(&first);
        assert!(matches!(
            reverify(&expected, &[moved.clone()], FilterOptions::default()),
            Err(OpenTargetError::IdentityChanged { .. })
        ));
        assert!(reverify(&expected, &[first], FilterOptions::default()).is_ok());
        let mut with_serial = usb();
        with_serial.location = Some("a".to_owned());
        let expected = TargetIdentity::from(&with_serial);
        with_serial.location = Some("b".to_owned());
        assert!(reverify(&expected, &[with_serial], FilterOptions::default()).is_ok());
    }

    #[test]
    fn une_cible_proposee_et_inchangee_passe() -> Result<(), OpenTargetError> {
        let disks = vec![usb()];
        let found = reverify(
            &TargetIdentity::from(&disks[0]),
            &disks,
            FilterOptions::default(),
        )?;
        assert_eq!(found.id, "usb-a");
        Ok(())
    }

    #[test]
    fn un_disque_debranche_est_introuvable() {
        let expected = TargetIdentity::from(&usb());
        let error = reverify(&expected, &[], FilterOptions::default()).err();
        assert!(matches!(error, Some(OpenTargetError::NotFound(id)) if id == "usb-a"));
    }

    #[test]
    fn un_disque_remplace_par_un_autre_au_meme_identifiant_est_refuse() {
        let expected = TargetIdentity::from(&usb());
        for replaced in [
            disk("usb-a", "S2", 1 << 33, Bus::Usb, vec![]),
            disk("usb-a", "S1", 1 << 34, Bus::Usb, vec![]),
        ] {
            let disks = vec![replaced];
            let error = reverify(&expected, &disks, FilterOptions::default()).err();
            assert!(
                matches!(error, Some(OpenTargetError::IdentityChanged { .. })),
                "{error:?}"
            );
        }
    }

    #[test]
    fn un_disque_devenu_interdit_est_refuse_meme_si_l_identite_est_la_meme() {
        let expected = TargetIdentity::from(&usb());
        // The key is now mounted as a system disk.
        let mut now = usb();
        now.partitions = vec![part("/dev/sdb1", &["/mnt/cle"], false)];
        let disks = vec![now];
        assert!(matches!(
            reverify(&expected, &disks, FilterOptions::default()),
            Err(OpenTargetError::Refused(Eligibility::Excluded { .. }))
        ));
        // An internal disk is writable only when the option allows it.
        let internal = disk("sata-x", "S9", 1 << 33, Bus::Sata, vec![]);
        let expected = TargetIdentity::from(&internal);
        let disks = vec![internal];
        assert!(matches!(
            reverify(&expected, &disks, FilterOptions::default()),
            Err(OpenTargetError::Refused(Eligibility::Hidden { .. }))
        ));
        let with_internal = FilterOptions {
            show_internal: true,
            ..FilterOptions::default()
        };
        assert!(reverify(&expected, &disks, with_internal).is_ok());
    }

    #[test]
    fn un_point_de_montage_repete_n_est_demonte_qu_une_fois() {
        let mut d = usb();
        d.partitions = vec![
            part("/dev/sdb1", &["/media/a"], false),
            part("/dev/sdb2", &["/media/b"], false),
            part("/dev/sdb3", &["/media/a"], false),
        ];
        assert_eq!(mount_points(&d), ["/media/a", "/media/b"]);
    }

    #[test]
    fn points_de_montage_du_plus_profond_au_moins_profond() {
        let mut d = usb();
        d.partitions = vec![
            part("/dev/sdb1", &["/run/media/u/A"], false),
            part("/dev/sdb2", &["/run/media/u/A/sous", "/media/x"], false),
        ];
        assert_eq!(
            mount_points(&d),
            ["/run/media/u/A/sous", "/run/media/u/A", "/media/x"]
        );
    }
}
