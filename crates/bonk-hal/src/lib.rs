//! Accès aux disques, propre à chaque système d'exploitation : énumération, filtre de sécurité,
//! accès brut, démontage, privilèges et formatage via les outils de l'OS.
//!
//! C'est le seul crate qui contient du code spécifique à un OS, isolé derrière `cfg(target_os)`
//! et une interface commune.

pub mod disk;
pub mod enumerate;
pub mod image;
#[cfg(target_os = "linux")]
pub mod linux;
pub mod safety;

pub use disk::{Bus, Disk, DiskKind, Partition};
pub use enumerate::{EnumerateError, enumerate_disks};
pub use image::{Access, FileDevice};
pub use safety::{Eligibility, Exclusion, FilterOptions, HiddenReason, Warning, evaluate};
