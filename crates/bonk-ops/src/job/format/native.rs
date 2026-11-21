use bonk_core::BlockDevice;
use bonk_core::format::table::PartitionPlan;
use bonk_core::format::{Filesystem, FormatError, exfat, fat32};

pub(super) fn native_cluster_bytes(
    filesystem: Filesystem,
    sectors: u64,
    block: u32,
) -> Result<Option<u32>, FormatError> {
    Ok(match filesystem {
        Filesystem::Fat32 => Some(fat32::geometry(sectors, block)?.cluster_bytes),
        Filesystem::Exfat => Some(exfat::geometry(sectors, block)?.cluster_bytes),
        Filesystem::Ntfs | Filesystem::Ext4 => None,
    })
}

pub(super) fn write_native(
    device: &mut dyn BlockDevice,
    filesystem: Filesystem,
    partition: PartitionPlan,
    label: Option<&str>,
    volume_id: u32,
) -> Result<(), FormatError> {
    match filesystem {
        Filesystem::Fat32 => fat32::write(
            device,
            &fat32::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
                bios_boot: false,
            },
        )
        .map(drop),
        Filesystem::Exfat => exfat::write(
            device,
            &exfat::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
            },
        )
        .map(drop),
        Filesystem::Ntfs | Filesystem::Ext4 => Ok(()),
    }
}

pub(super) fn verify_native(
    device: &dyn BlockDevice,
    filesystem: Filesystem,
    partition: PartitionPlan,
    label: Option<&str>,
    volume_id: u32,
) -> Result<(), FormatError> {
    match filesystem {
        Filesystem::Fat32 => fat32::verify(
            device,
            &fat32::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
                bios_boot: false,
            },
        ),
        Filesystem::Exfat => exfat::verify(
            device,
            &exfat::Volume {
                first_lba: partition.first_lba,
                sectors: partition.sectors(),
                label,
                volume_id,
            },
        ),
        Filesystem::Ntfs | Filesystem::Ext4 => Ok(()),
    }
}
