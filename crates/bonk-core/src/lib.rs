//! Cœur pur de Bonk : tables de partitions, formatage FAT32, écriture brute, décompression,
//! vérification, réparation du GPT et analyse d'image.
//!
//! Aucune entrée-sortie système, aucune API d'OS, aucun autre crate du workspace : tout se teste sur
//! des fichiers image, sans matériel ni privilèges.

pub mod analysis;
pub mod block;
pub mod codec;
pub mod format;
pub mod msc;
pub mod partition;
pub mod pe;
pub mod repair;
pub mod trace;
pub mod udf;
pub mod wim;
pub mod write;

pub use block::{
    BlockDevice, BlockError, DEFAULT_BLOCK_SIZE, MemoryDevice, check_access, check_geometry,
};
