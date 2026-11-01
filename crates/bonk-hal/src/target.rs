//! Choix et préparation d'une cible d'écriture.
//!
//! Trois garde-fous, dans cet ordre : le filtre de sécurité décide que la cible est écrivable ;
//! les partitions montées sont démontées ; **juste avant** l'ouverture, on ré-énumère et on vérifie que
//! c'est bien le même disque (identifiant, numéro de série, taille), pour qu'un disque débranché puis
//! remplacé entre-temps ne soit jamais écrit à la place de l'autre.

use std::fmt;

use crate::disk::Disk;
use crate::safety::{Eligibility, FilterOptions, evaluate};

/// Ce qui identifie un disque d'un branchement à l'autre.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIdentity {
    pub id: String,
    pub serial: Option<String>,
    pub size_bytes: u64,
}

impl From<&Disk> for TargetIdentity {
    fn from(disk: &Disk) -> Self {
        Self {
            id: disk.id.clone(),
            serial: disk.serial.clone(),
            size_bytes: disk.size_bytes,
        }
    }
}

#[derive(Debug)]
pub enum OpenTargetError {
    NotFound(String),
    /// Le filtre de sécurité refuse ou masque la cible.
    Refused(Eligibility),
    /// Ce n'est plus le même disque que celui qui a été choisi.
    IdentityChanged {
        expected: TargetIdentity,
        actual: TargetIdentity,
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

/// Retrouve la cible par son identifiant stable dans une énumération fraîche, la fait juger par le
/// filtre et vérifie son identité, sans rien toucher : appelée au choix puis **de nouveau** juste avant
/// l'ouverture. Seul le verdict « proposé » autorise la cible.
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
            expected: expected.clone(),
            actual,
        });
    }
    Ok(disk)
}

/// Points de montage de tout ce que porte le disque, du plus profond au moins profond.
#[must_use]
pub fn mount_points(disk: &Disk) -> Vec<String> {
    let mut mounts: Vec<String> = disk
        .partitions
        .iter()
        .flat_map(|partition| partition.mount_points.iter().cloned())
        .collect();
    // La valeur départage les longueurs égales : `dedup` ne retire que des voisins.
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
        // La clé est maintenant montée comme système (ou hors des chemins utilisateur).
        let mut now = usb();
        now.partitions = vec![part("/dev/sdb1", &["/mnt/cle"], false)];
        let disks = vec![now];
        assert!(matches!(
            reverify(&expected, &disks, FilterOptions::default()),
            Err(OpenTargetError::Refused(Eligibility::Excluded { .. }))
        ));
        // Un disque interne n'est écrivable que si l'option l'autorise.
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
