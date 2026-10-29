//! Filtre de sécurité des disques (CDC §3.1 et §3.2).
//!
//! [`evaluate`] est une **fonction pure** : elle ne lit que le [`Disk`] qu'on lui donne. Elle ne
//! touche ni le système ni le réseau, donc elle se teste sur des jeux de données capturés (voir
//! `tests/fixtures/`) et donne le même verdict sur les trois OS. C'est la seule porte vers
//! l'écriture : la CLI comme la GUI proposent un disque si et seulement si le verdict est
//! [`Eligibility::Offered`], et le revérifient juste avant d'écrire (CDC §3.3).
//!
//! # Parti pris : en cas de doute, exclure
//! Un disque refusé à tort se contourne (démontage, autre outil) ; un disque effacé à tort ne se
//! récupère pas. Toutes les règles vont donc dans le sens du refus : bus inconnu traité comme
//! interne, signature RAID/LVM/ZFS refusée même inactive, partition d'échange refusée même inutilisée,
//! montage hors des chemins utilisateur refusé même s'il est manuel (`/mnt/cle` : démonter d'abord).

use serde::{Deserialize, Serialize};

use crate::disk::{Bus, Disk, DiskKind, Partition};

/// Chemins de montage réservés à l'utilisateur sous Unix (CDC §3.1 point 2). Le montage doit être
/// **sous** ce chemin : `/media` lui-même ne compte pas.
const USER_MOUNT_PREFIXES: [&str; 3] = ["/media/", "/run/media/", "/Volumes/"];

/// Signatures de membres de RAID, LVM, ZFS et Storage Spaces (CDC §3.1 point 3), en minuscules. Les
/// adaptateurs d'OS ramènent leur vocabulaire à celui de `blkid`/`lsblk` ; Storage Spaces n'en a pas,
/// d'où `storage_spaces_member`.
const AGGREGATE_SIGNATURES: [&str; 7] = [
    "linux_raid_member",
    "ddf_raid_member",
    "isw_raid_member",
    "lvm2_member",
    "lvm1_member",
    "zfs_member",
    "storage_spaces_member",
];

/// Signatures d'espace d'échange ou de reprise de mise en veille (CDC §3.1 point 1).
const SWAP_SIGNATURES: [&str; 2] = ["swap", "swsuspend"];

/// Réglages explicites de l'utilisateur ; tout est refusé par défaut.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct FilterOptions {
    /// Afficher aussi les disques internes non système, avec un avertissement renforcé (CDC §3.2).
    pub show_internal: bool,
    /// Mode test explicite : autoriser les disques virtuels (loop, zram, disque de VM).
    pub allow_virtual: bool,
}

/// Avertissement attaché à un disque proposé.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Warning {
    None,
    /// Disque interne non système, affiché sur demande : confirmation renforcée.
    InternalDisk,
    /// Disque virtuel, autorisé par le mode test.
    VirtualDisk,
}

/// Pourquoi un disque non interdit n'est pas affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HiddenReason {
    /// Bus non amovible (ou inconnu) et l'option d'affichage des disques internes est désactivée.
    InternalBus,
}

/// Motif d'exclusion : le disque n'est jamais proposé ni écrivable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Exclusion {
    /// Volume système de l'hôte (racine, `%SystemDrive%`, volume de démarrage).
    SystemVolume { partition: String },
    /// Partition EFI dont l'hôte a démarré.
    ActiveEsp { partition: String },
    /// Espace d'échange ou de reprise de mise en veille.
    Swap { partition: String },
    /// Monté par le système hors des points de montage utilisateur.
    MountedOutsideUserPaths { partition: String, mount: String },
    /// Membre d'un RAID, LVM, ZFS ou Storage Spaces.
    AggregateMember { device: String, signature: String },
    /// Un autre périphérique est empilé dessus (RAID actif, LVM, volume chiffré déverrouillé).
    Stacked {
        device: String,
        holders: Vec<String>,
    },
    /// Disque virtuel de l'hôte, hors mode test.
    VirtualDisk { kind: DiskKind },
}

/// Verdict du filtre pour un disque.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Eligibility {
    /// Proposé à l'utilisateur et écrivable.
    Offered { warning: Warning },
    /// Pas interdit, mais non affiché avec les réglages actuels.
    Hidden { reason: HiddenReason },
    /// Jamais proposé, jamais écrivable. Tous les motifs sont listés, pour pouvoir les expliquer.
    Excluded { reasons: Vec<Exclusion> },
}

impl Eligibility {
    /// Vrai seulement pour un disque proposé : la garde à appeler avant toute écriture.
    #[must_use]
    pub fn is_writable(&self) -> bool {
        matches!(self, Self::Offered { .. })
    }
}

/// Décide du sort d'un disque.
#[must_use]
pub fn evaluate(disk: &Disk, options: FilterOptions) -> Eligibility {
    let mut reasons = Vec::new();

    if disk.kind != DiskKind::Physical && !options.allow_virtual {
        reasons.push(Exclusion::VirtualDisk { kind: disk.kind });
    }
    // Espace d'échange posé sur le disque entier, sans partition : la règle des partitions ne le voit pas.
    if disk
        .fs_type
        .as_deref()
        .is_some_and(|signature| is_one_of(signature, &SWAP_SIGNATURES))
    {
        reasons.push(Exclusion::Swap {
            partition: disk.path.clone(),
        });
    }
    push_device_exclusions(
        &mut reasons,
        &disk.path,
        disk.fs_type.as_deref(),
        &disk.holders,
    );
    for partition in &disk.partitions {
        push_partition_exclusions(&mut reasons, disk.bus, partition);
    }

    if !reasons.is_empty() {
        return Eligibility::Excluded { reasons };
    }
    if disk.bus.is_removable() {
        Eligibility::Offered {
            warning: Warning::None,
        }
    } else if options.allow_virtual && disk.bus == Bus::Virtual {
        Eligibility::Offered {
            warning: Warning::VirtualDisk,
        }
    } else if options.show_internal {
        Eligibility::Offered {
            warning: Warning::InternalDisk,
        }
    } else {
        Eligibility::Hidden {
            reason: HiddenReason::InternalBus,
        }
    }
}

/// Règles communes au disque entier et à ses partitions : signature d'agrégat et périphériques
/// empilés.
fn push_device_exclusions(
    reasons: &mut Vec<Exclusion>,
    device: &str,
    fs_type: Option<&str>,
    holders: &[String],
) {
    // Pas de `if let ... &&` : les chaînes de `let` exigent Rust 1.88, la version minimale est 1.85.
    match fs_type {
        Some(signature) if is_one_of(signature, &AGGREGATE_SIGNATURES) => {
            reasons.push(Exclusion::AggregateMember {
                device: device.to_owned(),
                signature: signature.to_owned(),
            });
        }
        _ => {}
    }
    if !holders.is_empty() {
        reasons.push(Exclusion::Stacked {
            device: device.to_owned(),
            holders: holders.to_vec(),
        });
    }
}

fn push_partition_exclusions(reasons: &mut Vec<Exclusion>, bus: Bus, partition: &Partition) {
    if partition.is_system_volume {
        reasons.push(Exclusion::SystemVolume {
            partition: partition.path.clone(),
        });
    }
    if partition.is_active_esp {
        reasons.push(Exclusion::ActiveEsp {
            partition: partition.path.clone(),
        });
    }
    if (partition.fs_type.as_deref())
        .is_some_and(|signature| is_one_of(signature, &SWAP_SIGNATURES))
    {
        reasons.push(Exclusion::Swap {
            partition: partition.path.clone(),
        });
    }
    push_device_exclusions(
        reasons,
        &partition.path,
        partition.fs_type.as_deref(),
        &partition.holders,
    );
    for mount in &partition.mount_points {
        if !is_user_mount(mount, bus) {
            reasons.push(Exclusion::MountedOutsideUserPaths {
                partition: partition.path.clone(),
                mount: mount.clone(),
            });
        }
    }
}

fn is_one_of(signature: &str, list: &[&str]) -> bool {
    list.iter()
        .any(|known| signature.eq_ignore_ascii_case(known))
}

/// Un point de montage est « utilisateur » s'il est sous `/media`, `/run/media` ou `/Volumes`, ou
/// s'il est la racine d'une lettre de lecteur d'un disque à bus amovible (CDC §3.1 point 2). Une
/// lettre sur un bus interne est un montage système : on ne distingue pas un disque de données d'un
/// disque d'exploitation à partir de la seule lettre.
fn is_user_mount(mount: &str, bus: Bus) -> bool {
    // Un point de montage réel est canonique ; un `..` ne peut être qu'une tentative de sortir de
    // l'arborescence utilisateur, ou une donnée corrompue : refuser dans les deux cas.
    if mount.split(['/', '\\']).any(|part| part == "..") {
        return false;
    }
    for prefix in USER_MOUNT_PREFIXES {
        if let Some(rest) = mount.strip_prefix(prefix) {
            return !rest.is_empty();
        }
    }
    is_drive_letter_root(mount) && bus.is_removable()
}

/// `E:`, `E:\` ou `E:/` : la racine d'une lettre de lecteur, pas un dossier monté sous une lettre.
fn is_drive_letter_root(mount: &str) -> bool {
    let bytes = mount.as_bytes();
    matches!(bytes, [letter, b':'] | [letter, b':', b'\\' | b'/'] if letter.is_ascii_alphabetic())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(path: &str) -> Partition {
        Partition {
            path: path.to_owned(),
            label: None,
            fs_type: None,
            size_bytes: 1 << 30,
            mount_points: vec![],
            holders: vec![],
            is_system_volume: false,
            is_active_esp: false,
        }
    }

    fn disk(bus: Bus, partitions: Vec<Partition>) -> Disk {
        Disk {
            id: "test-disk".to_owned(),
            path: "/dev/sdx".to_owned(),
            model: "Test".to_owned(),
            serial: None,
            size_bytes: 8 << 30,
            bus,
            kind: DiskKind::Physical,
            removable_flag: false,
            fs_type: None,
            holders: vec![],
            partitions,
        }
    }

    fn excluded(verdict: &Eligibility) -> &[Exclusion] {
        match verdict {
            Eligibility::Excluded { reasons } => reasons,
            autre => panic!("attendu Excluded, obtenu {autre:?}"),
        }
    }

    #[test]
    fn points_de_montage_utilisateur() {
        for mount in [
            "/media/alice/CLE",
            "/run/media/alice/CLE",
            "/Volumes/CLE",
            "/media/x",
        ] {
            assert!(is_user_mount(mount, Bus::Usb), "{mount}");
        }
    }

    #[test]
    fn points_de_montage_systeme() {
        for mount in [
            "/",
            "/boot",
            "/boot/efi",
            "/mnt/cle",
            "/media",
            "/media/",
            "/run/media",
            "/Volumes",
            "/mediafoo/x",
            "/home/alice/media/x",
            "/Media/x",
            "/media/../etc",
            "/media/alice/../../boot",
            "",
        ] {
            assert!(!is_user_mount(mount, Bus::Usb), "{mount:?}");
        }
    }

    #[test]
    fn lettres_de_lecteur() {
        for mount in ["E:", "E:\\", "e:/"] {
            assert!(is_user_mount(mount, Bus::Usb), "{mount}");
            assert!(is_user_mount(mount, Bus::Sd), "{mount}");
            assert!(!is_user_mount(mount, Bus::Sata), "{mount} sur bus interne");
            assert!(
                !is_user_mount(mount, Bus::Unknown),
                "{mount} sur bus inconnu"
            );
        }
        for mount in [
            "E:\\dossier",
            "E:\\..\\Windows",
            "EE:\\",
            "1:\\",
            "\\\\serveur\\partage",
        ] {
            assert!(!is_user_mount(mount, Bus::Usb), "{mount}");
        }
    }

    #[test]
    fn cle_usb_non_montee_est_proposee() {
        let verdict = evaluate(
            &disk(Bus::Usb, vec![part("/dev/sdx1")]),
            FilterOptions::default(),
        );
        assert_eq!(
            verdict,
            Eligibility::Offered {
                warning: Warning::None
            }
        );
        assert!(verdict.is_writable());
    }

    #[test]
    fn le_drapeau_amovible_ne_decide_pas() {
        let mut usb = disk(Bus::Usb, vec![]);
        usb.removable_flag = false;
        assert!(evaluate(&usb, FilterOptions::default()).is_writable());

        let mut interne = disk(Bus::Sata, vec![]);
        interne.removable_flag = true;
        assert_eq!(
            evaluate(&interne, FilterOptions::default()),
            Eligibility::Hidden {
                reason: HiddenReason::InternalBus
            }
        );
    }

    #[test]
    fn bus_inconnu_est_traite_comme_interne() {
        let verdict = evaluate(&disk(Bus::Unknown, vec![]), FilterOptions::default());
        assert!(!verdict.is_writable());
        let verdict = evaluate(
            &disk(Bus::Unknown, vec![]),
            FilterOptions {
                show_internal: true,
                ..FilterOptions::default()
            },
        );
        assert_eq!(
            verdict,
            Eligibility::Offered {
                warning: Warning::InternalDisk
            }
        );
    }

    #[test]
    fn tous_les_motifs_sont_listes() {
        let mut system = part("/dev/sdx1");
        system.is_system_volume = true;
        system.mount_points = vec!["/".to_owned()];
        let mut swap = part("/dev/sdx2");
        swap.fs_type = Some("swap".to_owned());
        let verdict = evaluate(
            &disk(Bus::Usb, vec![system, swap]),
            FilterOptions::default(),
        );
        assert_eq!(
            excluded(&verdict),
            [
                Exclusion::SystemVolume {
                    partition: "/dev/sdx1".to_owned()
                },
                Exclusion::MountedOutsideUserPaths {
                    partition: "/dev/sdx1".to_owned(),
                    mount: "/".to_owned()
                },
                Exclusion::Swap {
                    partition: "/dev/sdx2".to_owned()
                },
            ]
        );
    }

    #[test]
    fn une_exclusion_l_emporte_sur_les_options() {
        let mut system = part("/dev/sdx1");
        system.is_system_volume = true;
        let options = FilterOptions {
            show_internal: true,
            allow_virtual: true,
        };
        assert!(!evaluate(&disk(Bus::Usb, vec![system.clone()]), options).is_writable());
        assert!(!evaluate(&disk(Bus::Sata, vec![system]), options).is_writable());
    }

    #[test]
    fn signature_d_agregat_insensible_a_la_casse() {
        let mut member = part("/dev/sdx1");
        member.fs_type = Some("LVM2_member".to_owned());
        let verdict = evaluate(&disk(Bus::Usb, vec![member]), FilterOptions::default());
        assert!(matches!(
            excluded(&verdict),
            [Exclusion::AggregateMember { .. }]
        ));
    }

    #[test]
    fn disque_entier_membre_d_un_agregat_ou_empile() {
        let mut nu = disk(Bus::Usb, vec![]);
        nu.fs_type = Some("zfs_member".to_owned());
        assert!(!evaluate(&nu, FilterOptions::default()).is_writable());

        let mut empile = disk(Bus::Usb, vec![]);
        empile.holders = vec!["md0".to_owned()];
        assert!(!evaluate(&empile, FilterOptions::default()).is_writable());
    }

    #[test]
    fn disque_virtuel_exclu_sauf_mode_test() {
        let mut lo = disk(Bus::Virtual, vec![]);
        lo.kind = DiskKind::Loop;
        let verdict = evaluate(&lo, FilterOptions::default());
        assert_eq!(
            excluded(&verdict),
            [Exclusion::VirtualDisk {
                kind: DiskKind::Loop
            }]
        );
        let verdict = evaluate(
            &lo,
            FilterOptions {
                allow_virtual: true,
                ..FilterOptions::default()
            },
        );
        assert_eq!(
            verdict,
            Eligibility::Offered {
                warning: Warning::VirtualDisk
            }
        );
    }

    #[test]
    fn le_mode_test_n_ouvre_pas_les_disques_internes() {
        let verdict = evaluate(
            &disk(Bus::Sata, vec![]),
            FilterOptions {
                allow_virtual: true,
                ..FilterOptions::default()
            },
        );
        assert!(!verdict.is_writable());
    }
}
