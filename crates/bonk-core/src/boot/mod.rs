//! Code d'amorçage BIOS écrit par Bonk. Sources dans `boot/` à la racine du dépôt, assemblées par
//! `boot/assemble.sh` avec la seule chaîne Rust ; la CI réassemble et compare ces octets.

/// Code du MBR : les 440 premiers octets du disque, avant sa signature et la table des partitions.
pub const MBR: &[u8; 440] = include_bytes!("mbr.bin");
