use schemars::JsonSchema;
use serde::Serialize;

use crate::block::BlockDevice;
use crate::format::exfat::read::Reader;
use crate::format::table::{
    self, GUID_ESP, MBR_ESP, PartitionPlan, Slot, TableKind, gpt_type, mbr_type, usable_end,
};
use crate::format::{ALIGNMENT, Filesystem, FormatError, Ids, exfat, fat32};
use crate::partition::{ESP_GUID_TEXT, Layout, MbrEntry, Severity, inspect};

use super::*;

/// Data partition first, so systems that mount only the first partition of a removable drive show the
/// images; boot partition at the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
pub struct VaultPlan {
    pub table: TableKind,
    pub vault: PartitionPlan,
    pub efi: PartitionPlan,
}

/// FAT32 needs 65,525 clusters: 64 MiB with 512-byte sectors, about 257 MiB with 4 KiB ones.
pub(super) fn efi_sectors(block: u32) -> Result<u64, FormatError> {
    let block = u64::from(block);
    let mut bytes = EFI_MIN_BYTES;
    loop {
        match fat32::geometry(bytes / block, u32::try_from(block).unwrap_or(0)) {
            Ok(_) => return Ok(bytes / block),
            Err(FormatError::TooSmall(_)) if bytes < EFI_MAX_BYTES => bytes += ALIGNMENT,
            Err(err) => return Err(err),
        }
    }
}

pub fn plan(size: u64, block: u32, table: TableKind) -> Result<VaultPlan, FormatError> {
    let too_small =
        || FormatError::TooSmall(format!("drive of {size} bytes too small for a vault"));
    let end = usable_end(size, block, table)?.ok_or_else(too_small)?;
    let align = ALIGNMENT.div_ceil(u64::from(block));
    let efi_first = (end + 1)
        .checked_sub(efi_sectors(block)?)
        .ok_or_else(too_small)?
        / align
        * align;
    if efi_first <= align || (efi_first - align) * u64::from(block) < VAULT_MIN_BYTES {
        return Err(too_small());
    }
    let plan = VaultPlan {
        table,
        vault: PartitionPlan {
            first_lba: align,
            last_lba: efi_first - 1,
        },
        efi: PartitionPlan {
            first_lba: efi_first,
            last_lba: end,
        },
    };
    exfat::geometry(plan.vault.sectors(), block)?;
    fat32::geometry(plan.efi.sectors(), block)?;
    Ok(plan)
}

pub(super) fn exfat_volume(plan: &VaultPlan, ids: &Ids) -> exfat::Volume<'static> {
    exfat::Volume {
        first_lba: plan.vault.first_lba,
        sectors: plan.vault.sectors(),
        label: Some(VAULT_LABEL),
        volume_id: ids.volume_id,
    }
}

/// Wipes old tables and partition starts, then writes the table. The plan must come from [`plan`] for this
/// drive: it is computed again and compared. Flushing is the caller's job.
pub fn write_table(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
) -> Result<(), FormatError> {
    if self::plan(dev.size(), dev.block_size(), plan.table)? != *plan {
        return Err(FormatError::Mismatch(
            "vault plan made for another drive".to_owned(),
        ));
    }
    let slots = [
        Slot {
            part: plan.vault,
            gpt_type: gpt_type(Filesystem::Exfat),
            mbr_type: mbr_type(Filesystem::Exfat),
            guid: ids.partition_guid,
            name: Some(VAULT_LABEL),
        },
        Slot {
            part: plan.efi,
            gpt_type: GUID_ESP,
            mbr_type: MBR_ESP,
            guid: efi_ids(ids).0,
            name: Some(EFI_NAME),
        },
    ];
    table::write_slots(dev, plan.table, &slots, ids, false)
}

pub fn write_volumes(
    dev: &mut dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &[BootFile],
) -> Result<(), FormatError> {
    exfat::write(dev, &exfat_volume(plan, ids))?;
    write_boot(dev, plan, ids, boot)
}

pub fn verify(
    dev: &dyn BlockDevice,
    plan: &VaultPlan,
    ids: &Ids,
    boot: &[BootFile],
) -> Result<(), FormatError> {
    if detect(dev)? != Some(*plan) {
        return Err(FormatError::Mismatch(
            "vault table read back differently".to_owned(),
        ));
    }
    exfat::verify(dev, &exfat_volume(plan, ids))?;
    verify_boot(dev, plan, ids, boot)
}

pub fn detect(dev: &dyn BlockDevice) -> Result<Option<VaultPlan>, FormatError> {
    let layout = inspect(dev)?;
    if layout.logical_block_size != dev.block_size()
        || layout
            .findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
    {
        return Ok(None);
    }
    let Some(plan) = vault_partitions(&layout) else {
        return Ok(None);
    };
    match Reader::open(dev, plan.vault.first_lba, plan.vault.sectors()) {
        Ok(reader) if reader.label()?.as_deref() == Some(VAULT_LABEL) => Ok(Some(plan)),
        Ok(_) | Err(FormatError::Invalid(_)) => Ok(None),
        Err(err) => Err(err),
    }
}

pub(super) fn vault_partitions(layout: &Layout) -> Option<VaultPlan> {
    let (table, vault, efi) = match (&layout.gpt, &layout.mbr) {
        (Some(gpt), Some(mbr)) if mbr.protective => {
            let [vault, efi] = gpt.entries.as_slice() else {
                return None;
            };
            if efi.type_guid != ESP_GUID_TEXT {
                return None;
            }
            (
                TableKind::Gpt,
                (vault.first_lba, vault.last_lba),
                (efi.first_lba, efi.last_lba),
            )
        }
        (None, Some(mbr)) if !mbr.protective => {
            let [vault, efi] = mbr.entries.as_slice() else {
                return None;
            };
            if efi.kind != MBR_ESP {
                return None;
            }
            let span = |entry: &MbrEntry| {
                let first = u64::from(entry.start_lba);
                (first + u64::from(entry.sectors))
                    .checked_sub(1)
                    .map(|last| (first, last))
            };
            (TableKind::Mbr, span(vault)?, span(efi)?)
        }
        _ => return None,
    };
    if vault.1 < vault.0 || efi.1 < efi.0 {
        return None;
    }
    Some(VaultPlan {
        table,
        vault: PartitionPlan {
            first_lba: vault.0,
            last_lba: vault.1,
        },
        efi: PartitionPlan {
            first_lba: efi.0,
            last_lba: efi.1,
        },
    })
}
