use std::fmt;

use crate::eltorito::{BootImage, PLATFORM_EFI, boot_images};
use crate::partition::chs;
use crate::read_at::ReadAt;

pub const MBR_LEN: usize = 512;
const ENTRY: usize = 446;
const MBR_ESP: u8 = 0xEF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotHybrid {
    /// Something is already in the first sector: a boot sector or a partition table.
    SystemAreaUsed,
    NoEfiImage,
    SeveralEfiImages,
    /// Past the 2 TiB an MBR can address.
    ImageOutOfReach,
}

impl fmt::Display for NotHybrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SystemAreaUsed => "the ISO's first sector is not empty",
            Self::NoEfiImage => "the ISO has no EFI boot image",
            Self::SeveralEfiImages => "the ISO has several EFI boot images",
            Self::ImageOutOfReach => "the ISO's EFI boot image lies past what an MBR addresses",
        })
    }
}

/// The first sector to write once a DVD-only ISO is on the drive: an MBR in its empty system area names its
/// EFI boot image as an EFI system partition, as `isohybrid --uefi` does; every other byte stays as published.
///
/// # Errors
/// [`NotHybrid`]: what keeps this ISO from starting this way.
pub fn hybrid_mbr(source: &(impl ReadAt + ?Sized)) -> Result<[u8; MBR_LEN], NotHybrid> {
    let mut system_area = [0u8; MBR_LEN];
    if source.read_exact_at(0, &mut system_area).is_err() || system_area.iter().any(|b| *b != 0) {
        return Err(NotHybrid::SystemAreaUsed);
    }
    let efi: Vec<_> = boot_images(source)
        .into_iter()
        .filter(|image| image.platform == PLATFORM_EFI)
        .filter_map(|image| sized(source, image))
        .collect();
    let image = match efi.as_slice() {
        [] => return Err(NotHybrid::NoEfiImage),
        [one] => one,
        _ => return Err(NotHybrid::SeveralEfiImages),
    };
    let first = u32::try_from(image.first_sector).map_err(|_| NotHybrid::ImageOutOfReach)?;
    let sectors = u32::try_from(image.sectors).map_err(|_| NotHybrid::ImageOutOfReach)?;
    let mut mbr = [0u8; MBR_LEN];
    let partition = &mut mbr[ENTRY..ENTRY + 16];
    partition[1..4].copy_from_slice(&chs(image.first_sector));
    partition[4] = MBR_ESP;
    partition[5..8].copy_from_slice(&chs(image.first_sector + image.sectors - 1));
    partition[8..12].copy_from_slice(&first.to_le_bytes());
    partition[12..16].copy_from_slice(&sectors.to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    Ok(mbr)
}

/// The entry's 16-bit count cannot hold an image of 65536 sectors or more: authoring tools then write 0 or 1,
/// and the size is the one its FAT boot sector declares, wherever the image lies in the ISO.
fn sized(source: &(impl ReadAt + ?Sized), image: BootImage) -> Option<BootImage> {
    if image.sectors > 1 {
        return Some(image);
    }
    let mut boot = [0u8; 512];
    source
        .read_exact_at(image.first_sector.checked_mul(512)?, &mut boot)
        .ok()?;
    let sectors = fat_sectors(&boot)?;
    (sectors > 1).then_some(BootImage { sectors, ..image })
}

// BIOS parameter block, Microsoft FAT specification, section 3.1.
fn fat_sectors(boot: &[u8; 512]) -> Option<u64> {
    if boot[510..] != [0x55, 0xAA] {
        return None;
    }
    let bytes_per_sector = u64::from(u16::from_le_bytes([boot[11], boot[12]]));
    if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096) {
        return None;
    }
    let small = u64::from(u16::from_le_bytes([boot[19], boot[20]]));
    let total = if small == 0 {
        u64::from(u32::from_le_bytes([boot[32], boot[33], boot[34], boot[35]]))
    } else {
        small
    };
    Some(total * bytes_per_sector / 512)
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
