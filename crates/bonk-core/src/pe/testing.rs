use super::{COMMON_NAME_OID, OPTIONAL_PE32_PLUS};

pub const X86_64: u16 = super::MACHINE_X86_64;
pub const ARM64: u16 = super::MACHINE_ARM64;
pub const X86: u16 = super::MACHINE_X86;

pub fn pe(machine: u16, certificates: &[u8]) -> Vec<u8> {
    let mut file = vec![0u8; 512];
    file[..2].copy_from_slice(b"MZ");
    file[0x3C] = 0x40;
    file[0x40..0x44].copy_from_slice(b"PE\0\0");
    file[0x44..0x46].copy_from_slice(&machine.to_le_bytes());
    let optional = 0x44 + 20;
    file[optional..optional + 2].copy_from_slice(&OPTIONAL_PE32_PLUS.to_le_bytes());
    if !certificates.is_empty() {
        let security = optional + 112 + 32;
        file[security..security + 4].copy_from_slice(&512u32.to_le_bytes());
        file[security + 4..security + 8]
            .copy_from_slice(&(certificates.len() as u32).to_le_bytes());
        file.extend_from_slice(certificates);
    }
    file
}

pub fn name(text: &str) -> Vec<u8> {
    let mut der = COMMON_NAME_OID.to_vec();
    der.extend([0x0C, text.len() as u8]);
    der.extend(text.bytes());
    der
}
