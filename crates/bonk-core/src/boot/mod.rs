//! Code d'amorçage BIOS écrit par Bonk. Sources dans `boot/` à la racine du dépôt, assemblées par
//! `boot/assemble.sh` avec la seule chaîne Rust ; la CI réassemble et compare ces octets.

/// Code du MBR : les 440 premiers octets du disque, avant sa signature et la table des partitions.
pub const MBR: &[u8; 440] = include_bytes!("mbr.bin");

/// Secteur de démarrage FAT32 ; les octets 3 à 89 sont laissés à zéro pour le BPB du formatage.
pub const VBR: &[u8; 512] = include_bytes!("vbr.bin");

/// Second étage, lu par `VBR` à partir du secteur 9 de la zone réservée du volume.
pub const STAGE2: &[u8; 4096] = include_bytes!("stage2.bin");

/// Premier secteur du volume où `VBR` lit `STAGE2`.
pub const STAGE2_SECTOR: u32 = 9;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_secteur_fat32_saute_par_dessus_le_bpb_et_porte_sa_signature() {
        assert_eq!(&VBR[..3], &[0xeb, 0x58, 0x90]);
        assert!(VBR[3..0x5a].iter().all(|&octet| octet == 0));
        assert_eq!(&VBR[510..], &[0x55, 0xaa]);
    }

    #[test]
    fn le_second_etage_tient_dans_la_zone_reservee_libre() {
        let secteurs = STAGE2.len() as u32 / 512;
        assert_eq!(secteurs, 8);
        assert!(STAGE2_SECTOR + secteurs <= 32);
    }
}
