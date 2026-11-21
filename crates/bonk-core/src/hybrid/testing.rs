use crate::iso9660::testing::image;

pub const SECTOR: usize = 2048;
pub const EFI_BLOCK: u32 = 48;

#[must_use]
pub fn dvd_iso(efi_sectors: u16) -> Vec<u8> {
    let mut iso = image("DVD_ONLY", &[("EFI/BOOT/BOOTAA64.EFI", 1)]);
    let blocks = 64u32;
    iso.resize(blocks as usize * SECTOR, 0);
    let pvd = &mut iso[16 * SECTOR..17 * SECTOR];
    pvd[80..84].copy_from_slice(&blocks.to_le_bytes());
    pvd[84..88].copy_from_slice(&blocks.to_be_bytes());
    let record = &mut iso[17 * SECTOR..18 * SECTOR];
    record[1..6].copy_from_slice(b"CD001");
    record[6] = 1;
    record[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
    record[0x47..0x4B].copy_from_slice(&40u32.to_le_bytes());
    let terminator = &mut iso[18 * SECTOR..19 * SECTOR];
    terminator[0] = 255;
    terminator[1..6].copy_from_slice(b"CD001");
    let catalog = &mut iso[40 * SECTOR..41 * SECTOR];
    catalog[0] = 1;
    catalog[1] = crate::eltorito::PLATFORM_EFI;
    catalog[30] = 0x55;
    catalog[31] = 0xAA;
    catalog[32] = 0x88;
    catalog[38..40].copy_from_slice(&efi_sectors.to_le_bytes());
    catalog[40..44].copy_from_slice(&EFI_BLOCK.to_le_bytes());
    let efi = EFI_BLOCK as usize * SECTOR;
    iso[efi..efi + 512].fill(0xE5);
    iso
}

#[must_use]
pub fn dvd_iso_with_fat(fat: &[u8]) -> Vec<u8> {
    let sectors = u16::try_from(fat.len() / 512).unwrap_or(u16::MAX);
    let mut iso = dvd_iso(sectors);
    let at = EFI_BLOCK as usize * SECTOR;
    iso.resize(at + fat.len(), 0);
    iso[at..].copy_from_slice(fat);
    iso
}

pub const FAR_EFI_BLOCK: u32 = 640;
pub const FAR_EFI_SECTORS: u32 = 69_632;

#[must_use]
pub fn dvd_iso_with_far_efi() -> Vec<u8> {
    let mut iso = dvd_iso(0);
    let blocks = FAR_EFI_BLOCK + 8;
    iso.resize(blocks as usize * SECTOR, 0);
    let pvd = &mut iso[16 * SECTOR..17 * SECTOR];
    pvd[80..84].copy_from_slice(&blocks.to_le_bytes());
    pvd[84..88].copy_from_slice(&blocks.to_be_bytes());
    iso[40 * SECTOR + 40..40 * SECTOR + 44].copy_from_slice(&FAR_EFI_BLOCK.to_le_bytes());
    let fat = FAR_EFI_BLOCK as usize * SECTOR;
    let boot = &mut iso[fat..fat + 512];
    boot[0] = 0xEB;
    boot[11..13].copy_from_slice(&512u16.to_le_bytes());
    boot[32..36].copy_from_slice(&FAR_EFI_SECTORS.to_le_bytes());
    boot[510] = 0x55;
    boot[511] = 0xAA;
    iso
}
