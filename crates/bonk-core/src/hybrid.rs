use std::fmt;

use crate::eltorito::{PLATFORM_EFI, boot_images};

pub const MBR_LEN: usize = 512;
const ENTRY: usize = 446;
const MBR_ESP: u8 = 0xEF;
/// CHS fields past the 8 GB limit: firmware reads the LBA fields.
const CHS_PAST_LIMIT: [u8; 3] = [0xFE, 0xFF, 0xFF];

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
pub fn hybrid_mbr(head: &[u8]) -> Result<[u8; MBR_LEN], NotHybrid> {
    if head
        .get(..MBR_LEN)
        .is_none_or(|first| first.iter().any(|b| *b != 0))
    {
        return Err(NotHybrid::SystemAreaUsed);
    }
    let efi: Vec<_> = boot_images(head)
        .into_iter()
        .filter(|image| image.platform == PLATFORM_EFI && image.sectors > 1)
        .collect();
    let image = match efi.as_slice() {
        [] => return Err(NotHybrid::NoEfiImage),
        [one] => one,
        _ => return Err(NotHybrid::SeveralEfiImages),
    };
    let mut mbr = [0u8; MBR_LEN];
    let partition = &mut mbr[ENTRY..ENTRY + 16];
    partition[1..4].copy_from_slice(&CHS_PAST_LIMIT);
    partition[4] = MBR_ESP;
    partition[5..8].copy_from_slice(&CHS_PAST_LIMIT);
    let first = u32::try_from(image.first_sector).map_err(|_| NotHybrid::ImageOutOfReach)?;
    let sectors = u32::try_from(image.sectors).map_err(|_| NotHybrid::ImageOutOfReach)?;
    partition[8..12].copy_from_slice(&first.to_le_bytes());
    partition[12..16].copy_from_slice(&sectors.to_le_bytes());
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    Ok(mbr)
}

#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;
