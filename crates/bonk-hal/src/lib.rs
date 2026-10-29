//! Accès aux disques, propre à chaque système d'exploitation : énumération, filtre de sécurité,
//! accès brut, démontage, privilèges et formatage via les outils de l'OS.
//!
//! C'est le seul crate qui contient du code spécifique à un OS, isolé derrière `cfg(target_os)`
//! et une interface commune.

pub mod describe;
pub mod disk;
pub mod enumerate;
pub mod image;
pub mod interrupt;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod node;
pub mod safety;
pub mod target;

pub use describe::{
    bus_label, exclusion_label, format_size, kind_label, state_label, verdict_badge,
};
pub use disk::{Bus, Disk, DiskKind, Partition};
pub use enumerate::{
    EnumerateError, SourceRead, enumerate_disks, enumerate_disks_from_capture,
    enumerate_disks_recorded,
};
pub use image::{Access, FileDevice};
pub use node::BlockNode;
pub use safety::{
    Eligibility, Exclusion, FilterOptions, HiddenReason, RuleTrace, Warning, evaluate,
    evaluate_traced,
};
pub use target::{OpenTargetError, TargetIdentity, mount_points, reverify, unmount_all};
