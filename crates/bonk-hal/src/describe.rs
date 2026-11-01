//! Libellés lisibles des faits et des verdicts sur les disques.
//!
//! La ligne de commande (toujours en anglais) et la fenêtre (en français) affichent les mêmes faits :
//! écrits une seule fois par langue, ils ne peuvent pas diverger.

use crate::disk::{Bus, DiskKind};
use crate::safety::{Eligibility, Exclusion, HiddenReason, Warning};

/// Langue d'un libellé.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    En,
    /// Langue de développement de l'interface ; le lancement réel la choisit toujours explicitement.
    #[default]
    Fr,
}

impl Lang {
    #[must_use]
    pub fn pick(self, en: &'static str, fr: &'static str) -> &'static str {
        match self {
            Self::En => en,
            Self::Fr => fr,
        }
    }
}

/// Taille en unités décimales, comme l'indiquent les fabricants (une clé de 16 Go fait 16 000 000 000
/// octets ou un peu plus). Troncature, jamais d'arrondi vers le haut : une clé ne paraît pas plus grande
/// qu'elle n'est.
#[must_use]
pub fn format_size(bytes: u64, lang: Lang) -> String {
    let (units, separator, byte) = match lang {
        Lang::En => (["TB", "GB", "MB", "KB"], '.', "B"),
        Lang::Fr => (["To", "Go", "Mo", "Ko"], ',', "o"),
    };
    let factors = [1_000_000_000_000, 1_000_000_000, 1_000_000, 1_000];
    for (factor, unit) in factors.into_iter().zip(units) {
        if bytes >= factor {
            let tenths = bytes * 10 / factor;
            return format!("{}{separator}{} {unit}", tenths / 10, tenths % 10);
        }
    }
    format!("{bytes} {byte}")
}

#[must_use]
pub fn bus_label(bus: Bus, lang: Lang) -> &'static str {
    match bus {
        Bus::Usb => "usb",
        Bus::Sd => "sd",
        Bus::Thunderbolt => "thunderbolt",
        Bus::Sata => "sata",
        Bus::Nvme => "nvme",
        Bus::Scsi => "scsi",
        Bus::Virtual => lang.pick("virtual", "virtuel"),
        Bus::Unknown => lang.pick("unknown", "inconnu"),
    }
}

#[must_use]
pub fn kind_label(kind: DiskKind, lang: Lang) -> &'static str {
    match kind {
        DiskKind::Physical => lang.pick("physical", "physique"),
        DiskKind::Loop => "loop",
        DiskKind::Zram => "zram",
        DiskKind::HostVirtual => lang.pick("virtual machine", "machine virtuelle"),
    }
}

/// Phrase d'état d'un disque : proposé, non affiché ou interdit avec ses motifs.
#[must_use]
pub fn state_label(eligibility: &Eligibility, lang: Lang) -> String {
    match eligibility {
        Eligibility::Offered { warning } => match warning {
            Warning::None => lang.pick("offered", "proposé").to_owned(),
            Warning::InternalDisk => lang
                .pick(
                    "offered (internal disk: stronger confirmation)",
                    "proposé (disque interne : confirmation renforcée)",
                )
                .to_owned(),
            Warning::VirtualDisk => lang
                .pick(
                    "offered (virtual disk, test mode)",
                    "proposé (disque virtuel, mode test)",
                )
                .to_owned(),
        },
        Eligibility::Hidden { reason } => match reason {
            HiddenReason::InternalBus => lang
                .pick(
                    "hidden (internal bus; --show-internal to show it)",
                    "non affiché (bus interne ; --show-internal pour l'afficher)",
                )
                .to_owned(),
        },
        Eligibility::Excluded { reasons } => {
            let texts: Vec<String> = reasons.iter().map(|r| exclusion_label(r, lang)).collect();
            match lang {
                Lang::En => format!("forbidden: {}", texts.join("; ")),
                Lang::Fr => format!("interdit : {}", texts.join(" ; ")),
            }
        }
    }
}

/// Mot d'état court, en majuscules : le verdict ne repose jamais sur la couleur seule.
#[must_use]
pub fn verdict_badge(eligibility: &Eligibility, lang: Lang) -> &'static str {
    match eligibility {
        Eligibility::Offered {
            warning: Warning::None,
        } => lang.pick("OFFERED", "PROPOSÉ"),
        Eligibility::Offered { .. } => lang.pick("OFFERED - WARNING", "PROPOSÉ - ATTENTION"),
        Eligibility::Hidden { .. } => lang.pick("HIDDEN", "NON AFFICHÉ"),
        Eligibility::Excluded { .. } => lang.pick("FORBIDDEN", "INTERDIT"),
    }
}

#[must_use]
pub fn exclusion_label(reason: &Exclusion, lang: Lang) -> String {
    match (reason, lang) {
        (Exclusion::SystemVolume { partition }, Lang::En) => format!("system volume ({partition})"),
        (Exclusion::SystemVolume { partition }, Lang::Fr) => {
            format!("volume système ({partition})")
        }
        (Exclusion::ActiveEsp { partition }, Lang::En) => {
            format!("active EFI partition ({partition})")
        }
        (Exclusion::ActiveEsp { partition }, Lang::Fr) => {
            format!("partition EFI active ({partition})")
        }
        (Exclusion::Swap { partition }, Lang::En) => format!("swap space ({partition})"),
        (Exclusion::Swap { partition }, Lang::Fr) => format!("espace d'échange ({partition})"),
        (Exclusion::MountedOutsideUserPaths { partition, mount }, Lang::En) => {
            format!("{partition} mounted on {mount}")
        }
        (Exclusion::MountedOutsideUserPaths { partition, mount }, Lang::Fr) => {
            format!("{partition} monté sur {mount}")
        }
        (Exclusion::AggregateMember { device, signature }, Lang::En) => {
            format!("{device} is an aggregate member ({signature})")
        }
        (Exclusion::AggregateMember { device, signature }, Lang::Fr) => {
            format!("{device} membre d'un agrégat ({signature})")
        }
        (Exclusion::Stacked { device, holders }, Lang::En) => {
            format!("{device} used by {}", holders.join(", "))
        }
        (Exclusion::Stacked { device, holders }, Lang::Fr) => {
            format!("{device} utilisé par {}", holders.join(", "))
        }
        (Exclusion::VirtualDisk { kind }, Lang::En) => {
            format!("virtual disk ({})", kind_label(*kind, lang))
        }
        (Exclusion::VirtualDisk { kind }, Lang::Fr) => {
            format!("disque virtuel ({})", kind_label(*kind, lang))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tailles_lisibles() {
        assert_eq!(format_size(0, Lang::En), "0 B");
        assert_eq!(format_size(999, Lang::Fr), "999 o");
        assert_eq!(format_size(999_999, Lang::En), "999.9 KB");
        assert_eq!(format_size(999_999, Lang::Fr), "999,9 Ko");
        assert_eq!(format_size(1_000_000, Lang::En), "1.0 MB");
        assert_eq!(format_size(16_008_609_792, Lang::En), "16.0 GB");
        assert_eq!(format_size(16_008_609_792, Lang::Fr), "16,0 Go");
        assert_eq!(format_size(512_110_190_592, Lang::En), "512.1 GB");
        assert_eq!(format_size(2_000_398_934_016, Lang::Fr), "2,0 To");
        // Troncature, jamais d'arrondi vers le haut.
        assert_eq!(format_size(15_999_999_999, Lang::En), "15.9 GB");
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
            state_label(&verdict, Lang::En),
            "forbidden: system volume (/dev/sda1); swap space (/dev/sda2)"
        );
        assert_eq!(
            state_label(&verdict, Lang::Fr),
            "interdit : volume système (/dev/sda1) ; espace d'échange (/dev/sda2)"
        );
    }
}
