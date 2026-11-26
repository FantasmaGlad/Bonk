use sha2::{Digest, Sha256};

use crate::block::BlockDevice;
use crate::format::NewFile;
use crate::format::{FormatError, Ids, fat32, guid};

use super::*;

/// A file of the boot partition: path from its root, `/` separated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFile {
    pub path: String,
    pub data: Vec<u8>,
}

/// What a machine starts the vault from: the files of the boot partition, and GRUB for BIOS when the catalog
/// offers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VaultBoot {
    pub files: Vec<BootFile>,
    pub bios: Option<BiosGrub>,
}

impl VaultBoot {
    /// GRUB for BIOS counts 512-byte sectors: a drive with larger ones starts by UEFI only.
    #[must_use]
    pub fn bios_on(&self, block: u32) -> Option<&BiosGrub> {
        self.bios.as_ref().filter(|_| block as usize == SECTOR)
    }
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
pub fn boot_files(
    efi: &[(&str, Vec<u8>)],
    families: &[VaultFamily],
) -> Result<Vec<BootFile>, FormatError> {
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
        data: menu(families).into_bytes(),
    });
    files.push(BootFile {
        path: FAMILIES_PATH.to_owned(),
        data: serde_json::to_vec_pretty(families).unwrap_or_default(),
    });
    files.extend(INITRD_SCRIPTS.iter().map(|(path, script)| BootFile {
        path: (*path).to_owned(),
        data: script.as_bytes().to_vec(),
    }));
    Ok(files)
}

pub(super) fn boot_layout(
    dev_block: u32,
    volume: &fat32::Volume<'_>,
    files: &[BootFile],
) -> Result<fat32::tree::Layout, FormatError> {
    let new: Vec<NewFile> = files
        .iter()
        .map(|file| NewFile {
            path: file.path.split('/').map(str::to_owned).collect(),
            size: file.data.len() as u64,
        })
        .collect();
    fat32::tree::layout(volume, dev_block, &new, &[])
}

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

/// Writes the boot partition and GRUB for BIOS: the data partition and its ISO images are not touched.
///
/// # Errors
/// Write failure, boot files too large for the partition, or a GRUB for BIOS that does not fit before it.
pub fn write_boot(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &VaultBoot,
) -> Result<(), FormatError> {
    let volume = fat32_volume(plan, ids);
    let layout = boot_layout(dev.block_size(), &volume, &boot.files)?;
    layout.write_structures(dev)?;
    for (index, file) in boot.files.iter().enumerate() {
        layout.write_file(dev, index, &mut file.data.as_slice(), &mut |_| true)?;
    }
    layout.finish(dev, &volume)?;
    match boot.bios_on(dev.block_size()) {
        Some(grub) => write_bios(dev, plan, grub),
        None => Ok(()),
    }
}

/// The boot partition reads back with exactly these files, each at the place Bonk would write it, and GRUB for
/// BIOS as Bonk would link it. The volume id is not compared, as each preparation draws a new one.
///
/// # Errors
/// Read failure, or a GRUB for BIOS that does not fit before the first partition.
pub fn boot_matches(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    boot: &VaultBoot,
) -> Result<bool, FormatError> {
    let volume = fat32_volume(plan, &Ids::from_seed(b"compare"));
    let layout = boot_layout(dev.block_size(), &volume, &boot.files)?;
    for (index, file) in boot.files.iter().enumerate() {
        let expected = crate::write::hex(&Sha256::digest(&file.data));
        match layout.verify_file(dev, index, &expected, &mut |_| true) {
            Ok(()) => {}
            Err(FormatError::Mismatch(_)) => return Ok(false),
            Err(other) => return Err(other),
        }
    }
    match boot.bios_on(dev.block_size()) {
        Some(grub) => bios_matches(dev, plan, grub),
        None => Ok(true),
    }
}

/// The boot partition and GRUB for BIOS read back as written.
///
/// # Errors
/// Read failure or a difference.
pub fn verify_boot(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &VaultBoot,
) -> Result<(), FormatError> {
    let volume = fat32_volume(plan, ids);
    let layout = boot_layout(dev.block_size(), &volume, &boot.files)?;
    layout.verify_structures(dev, &volume)?;
    for (index, file) in boot.files.iter().enumerate() {
        let expected = crate::write::hex(&Sha256::digest(&file.data));
        layout.verify_file(dev, index, &expected, &mut |_| true)?;
    }
    match boot.bios_on(dev.block_size()) {
        Some(grub) if !bios_matches(dev, plan, grub)? => Err(FormatError::Mismatch(
            "GRUB for BIOS read back differently".to_owned(),
        )),
        _ => Ok(()),
    }
}
