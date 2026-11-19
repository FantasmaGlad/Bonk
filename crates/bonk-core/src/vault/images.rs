use crate::block::BlockDevice;
use crate::format::FormatError;
use crate::format::exfat;
use crate::format::exfat::read::{FileEntry, Reader};

use super::*;

/// Adds an ISO at the root of the vault's data partition, contiguous; the caller flushes then calls
/// [`verify_image`].
///
/// # Errors
/// See [`exfat::edit::add_file`].
pub fn add_image(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    name: &str,
    size: u64,
    family: IsoFamily,
    reader: &mut dyn std::io::Read,
    advance: &mut dyn FnMut(u64) -> bool,
) -> Result<exfat::edit::Added, FormatError> {
    let span = vault_span(plan);
    let added = exfat::edit::add_file(dev, span, name, size, false, reader, advance)?;
    if family.needs_location() {
        let location = location_file(added.volume_offset);
        let size = location.len() as u64;
        exfat::edit::add_file(
            dev,
            span,
            &format!("{name}{LOCATION_SUFFIX}"),
            size,
            true,
            &mut location.as_slice(),
            &mut |_| true,
        )?;
    }
    Ok(added)
}

/// Next to an ISO whose initramfs cannot read exFAT, hidden: where it starts on the data partition. The menu
/// passes this offset and Bonk's initrd step maps the ISO straight from the partition.
pub const LOCATION_SUFFIX: &str = ".bonk";

pub(super) fn vault_span(plan: &VaultPlan) -> exfat::edit::Span {
    exfat::edit::Span {
        first_lba: plan.vault.first_lba,
        sectors: plan.vault.sectors(),
    }
}

/// A GRUB script the menu sources.
pub(super) fn location_file(volume_offset: u64) -> Vec<u8> {
    format!("set bonk_offset={volume_offset}\n").into_bytes()
}

/// # Errors
/// See [`exfat::edit::verify_file`].
pub fn verify_image(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    name: &str,
    sha256: &str,
    advance: &mut dyn FnMut(u64) -> bool,
) -> Result<(), FormatError> {
    exfat::edit::verify_file(dev, vault_span(plan), name, sha256, advance)
}

/// Removes an ISO from the root of the data partition; returns its size.
///
/// # Errors
/// See [`exfat::edit::remove_file`].
pub fn remove_image(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    name: &str,
) -> Result<u64, FormatError> {
    let span = vault_span(plan);
    let freed = exfat::edit::remove_file(dev, span, name)?;
    // Only Ubuntu ISOs have one: its absence is the usual case.
    match exfat::edit::remove_file(dev, span, &format!("{name}{LOCATION_SUFFIX}")) {
        Ok(_) | Err(FormatError::Name(_)) => Ok(freed),
        Err(other) => Err(other),
    }
}

/// Files whose name ends in `.iso`, whatever the case, anywhere on the data partition.
pub fn images(dev: &dyn BlockDevice, plan: &VaultPlan) -> Result<Vec<FileEntry>, FormatError> {
    let files = Reader::open(dev, plan.vault.first_lba, plan.vault.sectors())?.files()?;
    Ok(files
        .into_iter()
        .filter(|file| {
            file.path
                .rsplit('/')
                .next()
                .is_some_and(|name| name.to_ascii_lowercase().ends_with(".iso"))
        })
        .collect())
}
