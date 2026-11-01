//! Filtre de sécurité des disques.
//!
//! [`evaluate`] est une **fonction pure** : elle ne lit que le [`Disk`] qu'on lui donne. Elle ne
//! touche ni le système ni le réseau, donc elle se teste sur des jeux de données capturés (voir
//! `tests/fixtures/`) et donne le même verdict sur les trois OS. C'est la seule porte vers
//! l'écriture : la CLI comme la GUI proposent un disque si et seulement si le verdict est
//! [`Eligibility::Offered`], et le revérifient juste avant d'écrire.
//!
//! # Parti pris : en cas de doute, exclure
//! Un disque refusé à tort se contourne (démontage, autre outil) ; un disque effacé à tort ne se
//! récupère pas. Toutes les règles vont donc dans le sens du refus : bus inconnu traité comme
//! interne, signature RAID/LVM/ZFS refusée même inactive, partition d'échange refusée même inutilisée,
//! montage hors des chemins utilisateur refusé même s'il est manuel (`/mnt/cle` : démonter d'abord).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::disk::{Bus, Disk, DiskKind, Partition};

/// Chemins de montage réservés à l'utilisateur sous Unix. Le montage doit être
/// **sous** ce chemin : `/media` lui-même ne compte pas.
const USER_MOUNT_PREFIXES: [&str; 3] = ["/media/", "/run/media/", "/Volumes/"];

/// Signatures de membres de RAID, LVM, ZFS et Storage Spaces, en minuscules. Les
/// adaptateurs d'OS ramènent leur vocabulaire à celui de `blkid`/`lsblk` ; Storage Spaces n'en a pas,
/// d'où `storage_spaces_member`.
const AGGREGATE_SIGNATURES: [&str; 8] = [
    "ldm_member",
    "linux_raid_member",
    "ddf_raid_member",
    "isw_raid_member",
    "lvm2_member",
    "lvm1_member",
    "zfs_member",
    "storage_spaces_member",
];

/// Signatures d'espace d'échange ou de reprise de mise en veille.
const SWAP_SIGNATURES: [&str; 2] = ["swap", "swsuspend"];

/// Explicit user settings; everything is refused by default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct FilterOptions {
    /// Show internal non-system disks too, with a stronger warning.
    pub show_internal: bool,
    /// Explicit test mode: allow virtual disks (loop, zram, VM disk).
    pub allow_virtual: bool,
}

/// Warning attached to an offered disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Warning {
    None,
    /// Internal non-system disk, shown on request: stronger confirmation.
    InternalDisk,
    /// Virtual disk, allowed by test mode.
    VirtualDisk,
}

/// Why a non-forbidden disk is hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HiddenReason {
    /// Non-removable (or unknown) bus, and the option to show internal disks is off.
    InternalBus,
}

/// Exclusion reason: the disk is never offered or writable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Exclusion {
    /// Host system volume (root, `%SystemDrive%`, boot volume).
    SystemVolume { partition: String },
    /// EFI partition the host booted from.
    ActiveEsp { partition: String },
    /// Swap or resume-from-sleep space.
    Swap { partition: String },
    /// Mounted by the system outside user mount points.
    MountedOutsideUserPaths { partition: String, mount: String },
    /// Member of a RAID, LVM, ZFS or Storage Spaces.
    AggregateMember { device: String, signature: String },
    /// Another device is stacked on it (active RAID, LVM, unlocked encrypted volume).
    Stacked {
        device: String,
        holders: Vec<String>,
    },
    /// Host virtual disk, outside test mode.
    VirtualDisk { kind: DiskKind },
}

/// Filter verdict for a disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Eligibility {
    /// Offered to the user and writable.
    Offered { warning: Warning },
    /// Not forbidden, but hidden with the current settings.
    Hidden { reason: HiddenReason },
    /// Never offered, never writable. All reasons are listed, so they can be explained.
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
    decide(disk, options, &mut Tracer(None))
}

/// Comme [`evaluate`], et rend aussi l'examen de chaque règle, appliquée ou non, avec les faits lus :
/// c'est ce que montre `bonk debug filter`. Le verdict est celui d'[`evaluate`], par construction (même
/// code) et par test.
#[must_use]
pub fn evaluate_traced(disk: &Disk, options: FilterOptions) -> (Eligibility, Vec<RuleTrace>) {
    let mut trace = Vec::new();
    let verdict = decide(disk, options, &mut Tracer(Some(&mut trace)));
    (verdict, trace)
}

/// Examination of a filter rule on a disk or partition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RuleTrace {
    /// Stable identifier of the rule (the same as the exclusion reason it produces).
    pub rule: &'static str,
    /// What is examined: path of the disk or partition.
    pub subject: String,
    /// The facts read by the rule.
    pub facts: String,
    /// The rule produced an exclusion (for `bus`, it decided whether the disk is offered).
    pub fired: bool,
}

/// Enregistreur facultatif : sans trace, aucun texte n'est construit.
struct Tracer<'t>(Option<&'t mut Vec<RuleTrace>>);

impl Tracer<'_> {
    fn note(
        &mut self,
        rule: &'static str,
        subject: &str,
        fired: bool,
        facts: impl FnOnce() -> String,
    ) {
        if let Some(trace) = self.0.as_mut() {
            trace.push(RuleTrace {
                rule,
                subject: subject.to_owned(),
                facts: facts(),
                fired,
            });
        }
    }
}

fn decide(disk: &Disk, options: FilterOptions, tracer: &mut Tracer<'_>) -> Eligibility {
    let mut reasons = Vec::new();

    let virtual_disk = disk.kind != DiskKind::Physical && !options.allow_virtual;
    tracer.note("virtual_disk", &disk.path, virtual_disk, || {
        format!(
            "kind={:?} allow_virtual={}",
            disk.kind, options.allow_virtual
        )
    });
    if virtual_disk {
        reasons.push(Exclusion::VirtualDisk { kind: disk.kind });
    }
    // Espace d'échange posé sur le disque entier, sans partition : la règle des partitions ne le voit pas.
    let swap = disk
        .fs_type
        .as_deref()
        .is_some_and(|signature| is_one_of(signature, &SWAP_SIGNATURES));
    tracer.note("swap", &disk.path, swap, || {
        format!("whole-disk signature={:?}", disk.fs_type)
    });
    if swap {
        reasons.push(Exclusion::Swap {
            partition: disk.path.clone(),
        });
    }
    push_device_exclusions(
        &mut reasons,
        tracer,
        &disk.path,
        disk.fs_type.as_deref(),
        &disk.holders,
    );
    for partition in &disk.partitions {
        push_partition_exclusions(&mut reasons, tracer, disk.bus, partition);
    }

    let verdict = if !reasons.is_empty() {
        Eligibility::Excluded { reasons }
    } else if disk.bus.is_removable() {
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
    };
    // La règle du bus ne s'applique qu'à un disque qu'aucune exclusion n'a écarté.
    let excluded = matches!(verdict, Eligibility::Excluded { .. });
    tracer.note("bus", &disk.path, !excluded, || {
        format!(
            "bus={:?} removable_bus={} show_internal={} allow_virtual={} -> {}",
            disk.bus,
            disk.bus.is_removable(),
            options.show_internal,
            options.allow_virtual,
            if excluded {
                "not reached (excluded above)"
            } else {
                crate::describe::verdict_badge(&verdict, crate::describe::Lang::En)
            }
        )
    });
    verdict
}

/// Règles communes au disque entier et à ses partitions : signature d'agrégat et périphériques
/// empilés.
fn push_device_exclusions(
    reasons: &mut Vec<Exclusion>,
    tracer: &mut Tracer<'_>,
    device: &str,
    fs_type: Option<&str>,
    holders: &[String],
) {
    let aggregate = fs_type.is_some_and(|signature| is_one_of(signature, &AGGREGATE_SIGNATURES));
    tracer.note("aggregate_member", device, aggregate, || {
        format!("signature={fs_type:?}")
    });
    if let (true, Some(signature)) = (aggregate, fs_type) {
        reasons.push(Exclusion::AggregateMember {
            device: device.to_owned(),
            signature: signature.to_owned(),
        });
    }
    tracer.note("stacked", device, !holders.is_empty(), || {
        format!("holders={holders:?}")
    });
    if !holders.is_empty() {
        reasons.push(Exclusion::Stacked {
            device: device.to_owned(),
            holders: holders.to_vec(),
        });
    }
}

fn push_partition_exclusions(
    reasons: &mut Vec<Exclusion>,
    tracer: &mut Tracer<'_>,
    bus: Bus,
    partition: &Partition,
) {
    let path = partition.path.as_str();
    tracer.note("system_volume", path, partition.is_system_volume, || {
        format!("is_system_volume={}", partition.is_system_volume)
    });
    if partition.is_system_volume {
        reasons.push(Exclusion::SystemVolume {
            partition: partition.path.clone(),
        });
    }
    tracer.note("active_esp", path, partition.is_active_esp, || {
        format!("is_active_esp={}", partition.is_active_esp)
    });
    if partition.is_active_esp {
        reasons.push(Exclusion::ActiveEsp {
            partition: partition.path.clone(),
        });
    }
    let swap = partition
        .fs_type
        .as_deref()
        .is_some_and(|signature| is_one_of(signature, &SWAP_SIGNATURES));
    tracer.note("swap", path, swap, || {
        format!("signature={:?}", partition.fs_type)
    });
    if swap {
        reasons.push(Exclusion::Swap {
            partition: partition.path.clone(),
        });
    }
    push_device_exclusions(
        reasons,
        tracer,
        &partition.path,
        partition.fs_type.as_deref(),
        &partition.holders,
    );
    if partition.mount_points.is_empty() {
        tracer.note("mounted_outside_user_paths", path, false, || {
            "not mounted".to_owned()
        });
    }
    for mount in &partition.mount_points {
        let outside = !is_user_mount(mount, bus);
        tracer.note("mounted_outside_user_paths", path, outside, || {
            format!("mount={mount:?} bus={bus:?} user_mount={}", !outside)
        });
        if outside {
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
/// s'il est la racine d'une lettre de lecteur d'un disque à bus amovible. Une
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

    fn fired(trace: &[RuleTrace]) -> Vec<(&'static str, &str)> {
        trace
            .iter()
            .filter(|t| t.fired && t.rule != "bus")
            .map(|t| (t.rule, t.subject.as_str()))
            .collect()
    }

    #[test]
    fn la_trace_donne_le_meme_verdict_et_liste_les_regles_appliquees() {
        let mut system = part("/dev/sdx1");
        system.is_system_volume = true;
        system.mount_points = vec!["/".to_owned()];
        let usb = disk(Bus::Usb, vec![system]);
        let options = FilterOptions::default();
        let (verdict, trace) = evaluate_traced(&usb, options);
        assert_eq!(verdict, evaluate(&usb, options));
        assert_eq!(
            fired(&trace),
            [
                ("system_volume", "/dev/sdx1"),
                ("mounted_outside_user_paths", "/dev/sdx1")
            ]
        );
        // Les règles examinées mais non appliquées figurent aussi, avec leurs faits.
        let esp = trace.iter().find(|t| t.rule == "active_esp");
        assert!(esp.is_some_and(|t| !t.fired && t.facts == "is_active_esp=false"));
        let mount = trace
            .iter()
            .find(|t| t.rule == "mounted_outside_user_paths");
        assert!(mount.is_some_and(
            |t| t.facts.contains("mount=\"/\"") && t.facts.contains("user_mount=false")
        ));
        // Un disque exclu n'atteint pas la règle du bus : elle le dit.
        let bus = trace.iter().find(|t| t.rule == "bus");
        assert!(bus.is_some_and(|t| !t.fired && t.facts.contains("not reached")));
    }

    #[test]
    fn la_trace_d_un_disque_propose_s_arrete_a_la_regle_du_bus() {
        let usb = disk(Bus::Usb, vec![part("/dev/sdx1")]);
        let (verdict, trace) = evaluate_traced(&usb, FilterOptions::default());
        assert!(verdict.is_writable());
        assert!(fired(&trace).is_empty());
        let bus = trace.last();
        assert!(bus.is_some_and(|t| t.rule == "bus" && t.fired && t.facts.contains("OFFERED")));
        // Une partition non montée est notée comme telle plutôt que passée sous silence.
        assert!(
            trace
                .iter()
                .any(|t| t.rule == "mounted_outside_user_paths" && t.facts == "not mounted")
        );
    }

    #[test]
    fn les_regles_ont_des_identifiants_stables_et_correspondent_aux_motifs() {
        let known = [
            "virtual_disk",
            "swap",
            "aggregate_member",
            "stacked",
            "system_volume",
            "active_esp",
            "mounted_outside_user_paths",
            "bus",
        ];
        let mut lvm = part("/dev/sdx1");
        lvm.fs_type = Some("LVM2_member".to_owned());
        lvm.holders = vec!["dm-0".to_owned()];
        let (_, trace) = evaluate_traced(&disk(Bus::Usb, vec![lvm]), FilterOptions::default());
        assert!(trace.iter().all(|t| known.contains(&t.rule)), "{trace:?}");
        assert_eq!(
            fired(&trace),
            [("aggregate_member", "/dev/sdx1"), ("stacked", "/dev/sdx1")]
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
