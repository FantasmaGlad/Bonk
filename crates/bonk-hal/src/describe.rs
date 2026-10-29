//! Libellés lisibles des faits et des verdicts sur les disques.
//!
//! La ligne de commande et l'interface graphique affichent les mêmes choses (taille, bus, état, motifs
//! d'exclusion) : les écrire une seule fois garantit qu'elles disent la même chose. Les textes sont en
//! français ; l'externalisation FR/EN exigée par le CDC §8 passera par ce module.

use crate::disk::{Bus, DiskKind};
use crate::safety::{Eligibility, Exclusion, HiddenReason, Warning};

/// Taille en unités décimales, comme l'indiquent les fabricants (une clé « 16 Go » fait 16 000 000 000
/// octets ou un peu plus), avec la virgule française. Troncature, jamais d'arrondi vers le haut : une
/// clé ne paraît pas plus grande qu'elle n'est.
#[must_use]
pub fn format_size(bytes: u64) -> String {
    const UNITS: [(u64, &str); 4] = [
        (1_000_000_000_000, "To"),
        (1_000_000_000, "Go"),
        (1_000_000, "Mo"),
        (1_000, "Ko"),
    ];
    for (factor, unit) in UNITS {
        if bytes >= factor {
            let tenths = bytes * 10 / factor;
            return format!("{},{} {unit}", tenths / 10, tenths % 10);
        }
    }
    format!("{bytes} o")
}

#[must_use]
pub fn bus_label(bus: Bus) -> &'static str {
    match bus {
        Bus::Usb => "usb",
        Bus::Sd => "sd",
        Bus::Thunderbolt => "thunderbolt",
        Bus::Sata => "sata",
        Bus::Nvme => "nvme",
        Bus::Scsi => "scsi",
        Bus::Virtual => "virtuel",
        Bus::Unknown => "inconnu",
    }
}

#[must_use]
pub fn kind_label(kind: DiskKind) -> &'static str {
    match kind {
        DiskKind::Physical => "physique",
        DiskKind::Loop => "loop",
        DiskKind::Zram => "zram",
        DiskKind::HostVirtual => "machine virtuelle",
    }
}

/// Phrase d'état d'un disque : « proposé », « non affiché... » ou « interdit : motifs ».
#[must_use]
pub fn state_label(eligibility: &Eligibility) -> String {
    match eligibility {
        Eligibility::Offered { warning } => match warning {
            Warning::None => "proposé".to_owned(),
            Warning::InternalDisk => "proposé (disque interne : confirmation renforcée)".to_owned(),
            Warning::VirtualDisk => "proposé (disque virtuel, mode test)".to_owned(),
        },
        Eligibility::Hidden { reason } => match reason {
            HiddenReason::InternalBus => {
                "non affiché (bus interne ; --show-internal pour l'afficher)".to_owned()
            }
        },
        Eligibility::Excluded { reasons } => {
            let texts: Vec<String> = reasons.iter().map(exclusion_label).collect();
            format!("interdit : {}", texts.join(" ; "))
        }
    }
}

/// Mot d'état court, en majuscules : le verdict ne repose jamais sur la couleur seule (CDC §8).
#[must_use]
pub fn verdict_badge(eligibility: &Eligibility) -> &'static str {
    match eligibility {
        Eligibility::Offered {
            warning: Warning::None,
        } => "PROPOSÉ",
        Eligibility::Offered { .. } => "PROPOSÉ - ATTENTION",
        Eligibility::Hidden { .. } => "NON AFFICHÉ",
        Eligibility::Excluded { .. } => "INTERDIT",
    }
}

#[must_use]
pub fn exclusion_label(reason: &Exclusion) -> String {
    match reason {
        Exclusion::SystemVolume { partition } => format!("volume système ({partition})"),
        Exclusion::ActiveEsp { partition } => format!("partition EFI active ({partition})"),
        Exclusion::Swap { partition } => format!("espace d'échange ({partition})"),
        Exclusion::MountedOutsideUserPaths { partition, mount } => {
            format!("{partition} monté sur {mount}")
        }
        Exclusion::AggregateMember { device, signature } => {
            format!("{device} membre d'un agrégat ({signature})")
        }
        Exclusion::Stacked { device, holders } => {
            format!("{device} utilisé par {}", holders.join(", "))
        }
        Exclusion::VirtualDisk { kind } => format!("disque virtuel ({})", kind_label(*kind)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailles_lisibles() {
        assert_eq!(format_size(0), "0 o");
        assert_eq!(format_size(999), "999 o");
        assert_eq!(format_size(999_999), "999,9 Ko");
        assert_eq!(format_size(1_000_000), "1,0 Mo");
        assert_eq!(format_size(16_008_609_792), "16,0 Go");
        assert_eq!(format_size(512_110_190_592), "512,1 Go");
        assert_eq!(format_size(2_000_398_934_016), "2,0 To");
        // Troncature, jamais d'arrondi vers le haut.
        assert_eq!(format_size(15_999_999_999), "15,9 Go");
    }

    #[test]
    fn etat_d_un_disque_interdit_liste_tous_les_motifs() {
        let verdict = Eligibility::Excluded {
            reasons: vec![
                Exclusion::SystemVolume {
                    partition: "/dev/sda1".to_owned(),
                },
                Exclusion::Swap {
                    partition: "/dev/sda2".to_owned(),
                },
            ],
        };
        assert_eq!(
            state_label(&verdict),
            "interdit : volume système (/dev/sda1) ; espace d'échange (/dev/sda2)"
        );
    }
}
