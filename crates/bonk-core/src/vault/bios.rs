use crate::block::BlockDevice;
use crate::format::FormatError;
use crate::format::table::TableKind;
use crate::partition::inspect;

use super::VaultPlan;

// GRUB 2.14, include/grub/i386/pc/boot.h and util/setup.c, as grub-bios-setup lays them out.
pub(super) const SECTOR: usize = 512;
const BPB: std::ops::Range<usize> = 0x03..0x5A;
const KERNEL_SECTOR: usize = 0x5C;
const BOOT_DRIVE: usize = 0x64;
const DRIVE_CHECK: usize = 0x66;
const TABLE: std::ops::Range<usize> = 0x1B8..0x1FE;
const LIST_SIZE: usize = 12;
const KERNEL_SEGMENT: u16 = 0x820;
const FIRST_ENTRY: usize = 0x1BE;
const ACTIVE: u8 = 0x80;

/// GRUB for BIOS machines: `boot.img` goes into the MBR, `core.img` right after the partition table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BiosGrub {
    pub boot: Vec<u8>,
    pub core: Vec<u8>,
}

// Linked as grub-bios-setup links them: boot.img names the first sector of core.img, whose block list names
// the rest.
fn bios_sectors(
    grub: &BiosGrub,
    sector0: &[u8; SECTOR],
    first_lba: u64,
    end_lba: u64,
) -> Result<([u8; SECTOR], Vec<u8>), FormatError> {
    let refused = |why: &str| Err(FormatError::Unsupported(format!("GRUB for BIOS: {why}")));
    if grub.boot.len() != SECTOR || grub.boot[510..] != [0x55, 0xAA] {
        return refused("boot.img is not a 512-byte boot sector");
    }
    if grub.core.len() < SECTOR {
        return refused("core.img is shorter than a sector");
    }
    let sectors = grub.core.len().div_ceil(SECTOR);
    let Ok(rest) = u16::try_from(sectors - 1) else {
        return refused("core.img is larger than its block list can name");
    };
    if first_lba == 0 || first_lba + sectors as u64 > end_lba {
        return refused("core.img does not fit before the first partition");
    }
    let mut mbr = [0u8; SECTOR];
    mbr.copy_from_slice(&grub.boot);
    mbr[BPB].copy_from_slice(&sector0[BPB]);
    mbr[KERNEL_SECTOR..KERNEL_SECTOR + 8].copy_from_slice(&first_lba.to_le_bytes());
    mbr[BOOT_DRIVE] = 0xFF;
    // A hard disk: the boot drive the BIOS passes is used even when a buggy BIOS reports 0x00.
    mbr[DRIVE_CHECK..DRIVE_CHECK + 2].copy_from_slice(&[0x90, 0x90]);
    mbr[TABLE].copy_from_slice(&sector0[TABLE]);
    let mut core = grub.core.clone();
    core.resize(sectors * SECTOR, 0);
    let mut at = SECTOR - LIST_SIZE;
    while core[at + 8..at + 10] != [0, 0] {
        core[at..at + LIST_SIZE].fill(0);
        if at < LIST_SIZE {
            return refused("core.img has no block list terminator");
        }
        at -= LIST_SIZE;
    }
    let first = SECTOR - LIST_SIZE;
    core[first..first + 8].copy_from_slice(&(first_lba + 1).to_le_bytes());
    core[first + 8..first + 10].copy_from_slice(&rest.to_le_bytes());
    core[first + 10..first + 12].copy_from_slice(&KERNEL_SEGMENT.to_le_bytes());
    Ok((mbr, core))
}

pub(super) fn write_bios(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    grub: &BiosGrub,
) -> Result<(), FormatError> {
    let (first, mbr, core) = expected(dev, plan, grub)?;
    dev.write_at(first * SECTOR as u64, &core)?;
    dev.write_at(0, &mbr)?;
    Ok(())
}

pub(super) fn bios_matches(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    grub: &BiosGrub,
) -> Result<bool, FormatError> {
    let (first, mbr, core) = expected(dev, plan, grub)?;
    let mut on_drive = vec![0u8; SECTOR + core.len()];
    dev.read_at(0, &mut on_drive[..SECTOR])?;
    dev.read_at(first * SECTOR as u64, &mut on_drive[SECTOR..])?;
    Ok(on_drive[..SECTOR] == mbr && on_drive[SECTOR..] == core)
}

fn expected(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    grub: &BiosGrub,
) -> Result<(u64, [u8; SECTOR], Vec<u8>), FormatError> {
    let first = core_lba(dev, plan.table)?;
    let mut sector0 = [0u8; SECTOR];
    dev.read_at(0, &mut sector0)?;
    let (mut mbr, core) = bios_sectors(grub, &sector0, first, plan.vault.first_lba)?;
    if plan.table == TableKind::Mbr {
        // Some BIOSes start a USB drive only when one of its partitions is marked active.
        mbr[FIRST_ENTRY] = ACTIVE;
    }
    Ok((first, mbr, core))
}

fn core_lba(dev: &dyn BlockDevice, table: TableKind) -> Result<u64, FormatError> {
    match table {
        TableKind::Mbr => Ok(1),
        TableKind::Gpt => inspect(dev)?
            .gpt
            .and_then(|gpt| gpt.primary)
            .map(|header| header.first_usable_lba)
            .ok_or_else(|| FormatError::Mismatch("vault GPT header unreadable".to_owned())),
    }
}

#[cfg(test)]
mod tests;
