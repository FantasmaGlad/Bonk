use sha2::{Digest, Sha256};

use crate::block::BlockDevice;
use crate::format::{FormatError, Ids, fat32, guid};
use crate::format::{NewFile, fat32_tree};

use super::*;

/// A file of the boot partition: path from its root, `/` separated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// Removable-media name of an architecture's EFI image: firmware loads the one of its own processor.
#[must_use]
pub fn efi_file_name(arch: &str) -> Option<&'static str> {
    match arch {
        "x86_64" => Some("BOOTX64.EFI"),
        "aarch64" => Some("BOOTAA64.EFI"),
        "riscv64" => Some("BOOTRISCV64.EFI"),
        _ => None,
    }
}

/// GRUB images by architecture, then the menu and the files it uses.
///
/// # Errors
/// [`FormatError::Unsupported`] for an architecture without an EFI removable-media name.
pub fn boot_files(efi: &[(&str, Vec<u8>)]) -> Result<Vec<BootFile>, FormatError> {
    let mut files = efi
        .iter()
        .map(|(arch, data)| {
            let name = efi_file_name(arch)
                .ok_or_else(|| FormatError::Unsupported(format!("no EFI boot file for {arch}")))?;
            Ok(BootFile {
                path: format!("EFI/BOOT/{name}"),
                data: data.clone(),
            })
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    files.push(BootFile {
        path: MENU_PATH.to_owned(),
        data: MENU.as_bytes().to_vec(),
    });
    files.push(BootFile {
        path: CASPER_EXFAT_PATH.to_owned(),
        data: CASPER_EXFAT.as_bytes().to_vec(),
    });
    Ok(files)
}

pub(super) fn boot_layout(
    dev_block: u32,
    volume: &fat32::Volume<'_>,
    files: &[BootFile],
) -> Result<fat32_tree::Layout, FormatError> {
    let new: Vec<NewFile> = files
        .iter()
        .map(|file| NewFile {
            path: file.path.split('/').map(str::to_owned).collect(),
            size: file.data.len() as u64,
        })
        .collect();
    fat32_tree::layout(volume, dev_block, &new, &[])
}

/// GUID and volume id of the boot partition, derived from the drive ids.
pub(super) fn efi_ids(ids: &Ids) -> ([u8; 16], u32) {
    let digest = Sha256::digest([&ids.partition_guid[..], b"efi"].concat());
    (
        guid(&digest[..16]),
        u32::from_le_bytes([digest[16], digest[17], digest[18], digest[19]]),
    )
}

pub(super) fn fat32_volume(plan: &VaultPlan, ids: &Ids) -> fat32::Volume<'static> {
    fat32::Volume {
        first_lba: plan.efi.first_lba,
        sectors: plan.efi.sectors(),
        label: Some(EFI_LABEL),
        volume_id: efi_ids(ids).1,
        bios_boot: false,
    }
}

/// Writes the boot partition alone: the data partition and its ISO images are not touched.
///
/// # Errors
/// Write failure, or boot files too large for the partition.
pub fn write_boot(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &[BootFile],
) -> Result<(), FormatError> {
    let volume = fat32_volume(plan, ids);
    let layout = boot_layout(dev.block_size(), &volume, boot)?;
    layout.write_structures(dev)?;
    for (index, file) in boot.iter().enumerate() {
        layout.write_file(dev, index, &mut file.data.as_slice(), &mut |_| true)?;
    }
    layout.finish(dev, &volume)
}

/// The boot partition reads back with exactly these files: each at the place Bonk would write it, with the same
/// bytes. Its volume id is not compared, as each preparation draws a new one.
///
/// # Errors
/// Read failure.
pub fn boot_matches(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    boot: &[BootFile],
) -> Result<bool, FormatError> {
    let volume = fat32_volume(plan, &Ids::from_seed(b"compare"));
    let layout = boot_layout(dev.block_size(), &volume, boot)?;
    for (index, file) in boot.iter().enumerate() {
        let expected = crate::write::hex(&Sha256::digest(&file.data));
        match layout.verify_file(dev, index, &expected, &mut |_| true) {
            Ok(()) => {}
            Err(FormatError::Mismatch(_)) => return Ok(false),
            Err(other) => return Err(other),
        }
    }
    Ok(true)
}

/// The boot partition alone read back as written.
///
/// # Errors
/// Read failure or a difference.
pub fn verify_boot(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &[BootFile],
) -> Result<(), FormatError> {
    let volume = fat32_volume(plan, ids);
    let layout = boot_layout(dev.block_size(), &volume, boot)?;
    layout.verify_structures(dev, &volume)?;
    for (index, file) in boot.iter().enumerate() {
        let expected = crate::write::hex(&Sha256::digest(&file.data));
        layout.verify_file(dev, index, &expected, &mut |_| true)?;
    }
    Ok(())
}
