use std::fmt;

use crate::efi::volume_loaders;
use crate::eltorito::{BootImage, PLATFORM_EFI, boot_images};
use crate::partition::chs;
use crate::read_at::ReadAt;

pub const MBR_LEN: usize = 512;
const ENTRY: usize = 446;
const MBR_ESP: u8 = 0xEF;
const MBR_SLOTS: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotHybrid {
    SystemAreaUsed,
    NoEfiImage,
    SeveralEfiImages,
    TooManyEfiImages,
    OverlappingEfiImages,
    ImageOutOfReach,
}

impl fmt::Display for NotHybrid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SystemAreaUsed => "the ISO's first sector is not empty",
            Self::NoEfiImage => "the ISO has no EFI boot image",
            Self::SeveralEfiImages => {
                "the ISO has several EFI boot images and none holds a removable-media loader"
            }
            Self::TooManyEfiImages => {
                "the ISO has more EFI boot images than the four an MBR declares"
            }
            Self::OverlappingEfiImages => "the ISO's EFI boot images overlap",
            Self::ImageOutOfReach => "the ISO's EFI boot image lies past what an MBR addresses",
        })
    }
}

pub fn hybrid_mbr(source: &(impl ReadAt + ?Sized)) -> Result<[u8; MBR_LEN], NotHybrid> {
    let mut system_area = [0u8; MBR_LEN];
    if source.read_exact_at(0, &mut system_area).is_err() || system_area.iter().any(|b| *b != 0) {
        return Err(NotHybrid::SystemAreaUsed);
    }
    let images = efi_images(source)?;
    let mut mbr = [0u8; MBR_LEN];
    for (slot, image) in images.iter().enumerate() {
        let first = u32::try_from(image.first_sector).map_err(|_| NotHybrid::ImageOutOfReach)?;
        let sectors = u32::try_from(image.sectors).map_err(|_| NotHybrid::ImageOutOfReach)?;
        let at = ENTRY + slot * 16;
        let partition = &mut mbr[at..at + 16];
        partition[1..4].copy_from_slice(&chs(image.first_sector));
        partition[4] = MBR_ESP;
        partition[5..8].copy_from_slice(&chs(image.first_sector + image.sectors - 1));
        partition[8..12].copy_from_slice(&first.to_le_bytes());
        partition[12..16].copy_from_slice(&sectors.to_le_bytes());
    }
    mbr[510] = 0x55;
    mbr[511] = 0xAA;
    Ok(mbr)
}

// Several images, one per architecture: each that holds a loader becomes its own EFI partition.
fn efi_images(source: &(impl ReadAt + ?Sized)) -> Result<Vec<BootImage>, NotHybrid> {
    let mut images: Vec<BootImage> = boot_images(source)
        .into_iter()
        .filter(|image| image.platform == PLATFORM_EFI)
        .filter_map(|image| sized(source, image))
        .collect();
    images.sort_by_key(|image| image.first_sector);
    images.dedup();
    if images.len() > 1 {
        images.retain(|image| {
            image
                .first_sector
                .checked_mul(512)
                .is_some_and(|start| !volume_loaders(source, start).is_empty())
        });
        if images.is_empty() {
            return Err(NotHybrid::SeveralEfiImages);
        }
    }
    match images.len() {
        0 => Err(NotHybrid::NoEfiImage),
        count if count > MBR_SLOTS => Err(NotHybrid::TooManyEfiImages),
        _ if images
            .windows(2)
            .any(|pair| pair[0].first_sector + pair[0].sectors > pair[1].first_sector) =>
        {
            Err(NotHybrid::OverlappingEfiImages)
        }
        _ => Ok(images),
    }
}

// An El Torito entry counts sectors on 16 bits: past 65535, its FAT boot sector gives the size.
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
