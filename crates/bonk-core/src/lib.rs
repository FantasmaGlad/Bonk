//! Cœur pur de Bonk : partitionnement MBR/GPT, écriture brute, décompression, vérification et
//! réparation du GPT de secours.
//!
//! Ce crate n'opère que sur une abstraction de périphérique bloc : aucune entrée-sortie système,
//! aucune API d'OS, aucune dépendance vers un autre crate du workspace. Il reste ainsi entièrement
//! testable sur des fichiers image, sans matériel ni privilèges.

pub mod analysis;
pub mod block;
pub mod codec;
pub mod format;
pub mod partition;
pub mod repair;
pub mod trace;
pub mod write;

pub use block::{
    BlockDevice, BlockError, DEFAULT_BLOCK_SIZE, MemoryDevice, check_access, check_geometry,
};
