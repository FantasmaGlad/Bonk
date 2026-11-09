//! BIOS boot code written by Bonk. Sources in `boot/`, assembled by `boot/assemble.sh` with the Rust
//! toolchain only; CI reassembles and compares these bytes.

/// First 440 bytes of the disk, before the signature and the partition table.
pub const MBR: &[u8; 440] = include_bytes!("mbr.bin");

/// FAT32 boot sector; bytes 3 to 89 stay zero for the BPB.
pub const VBR: &[u8; 512] = include_bytes!("vbr.bin");

/// Second stage, read by `VBR` from the reserved area.
pub const STAGE2: &[u8; 4096] = include_bytes!("stage2.bin");

pub const STAGE2_SECTOR: u32 = 9;

/// Sector size that `VBR` and `STAGE2` assume.
pub const SECTOR: u32 = 512;

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
