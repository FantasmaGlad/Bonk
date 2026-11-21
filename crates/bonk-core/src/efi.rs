use schemars::JsonSchema;
use serde::Serialize;

use crate::analysis::head_layout;
use crate::eltorito::{PLATFORM_EFI, boot_images};
use crate::fat::Fat;
use crate::partition::{ESP_GUID_TEXT, Layout, MBR_ESP};
use crate::pe::{self, Machine, Signer};
use crate::read_at::ReadAt;

pub const LOADER_LIMIT: u64 = 64 << 20;
// Removable-media file names, UEFI specification, section 3.5.1.1.
pub const LOADERS: [(&str, Machine); 3] = [
    ("BOOTX64.EFI", Machine::X86_64),
    ("BOOTAA64.EFI", Machine::Arm64),
    ("BOOTIA32.EFI", Machine::X86),
];
// Firmware looks for the removable-media loader on any FAT partition, not only on type 0xEF.
const MBR_FAT: [u8; 7] = [0x01, 0x04, 0x06, 0x0B, 0x0C, 0x0E, MBR_ESP];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Loader {
    pub machine: Machine,
    pub signer: Signer,
}

#[must_use]
pub fn loaders(source: &(impl ReadAt + ?Sized), head: &[u8]) -> Vec<Loader> {
    let mut found: Vec<Loader> = Vec::new();
    for start in volumes(source, head) {
        for loader in volume_loaders(source, start) {
            if !found.iter().any(|known| known.machine == loader.machine) {
                found.push(loader);
            }
        }
    }
    found
}

#[must_use]
pub fn volume_loaders(source: &(impl ReadAt + ?Sized), start: u64) -> Vec<Loader> {
    let Some(fat) = Fat::open(source, start) else {
        return Vec::new();
    };
    LOADERS
        .into_iter()
        .filter_map(|(name, machine)| {
            let bytes = fat.read(&format!("EFI/BOOT/{name}"), LOADER_LIMIT)?;
            (pe::machine(&bytes) == Some(machine)).then(|| Loader {
                machine,
                signer: pe::signer(&bytes).unwrap_or(Signer::Unsigned),
            })
        })
        .collect()
}

fn volumes(source: &(impl ReadAt + ?Sized), head: &[u8]) -> Vec<u64> {
    let mut starts: Vec<u64> = head_layout(head)
        .as_ref()
        .map(partitions)
        .unwrap_or_default();
    starts.extend(
        boot_images(source)
            .into_iter()
            .filter(|image| image.platform == PLATFORM_EFI)
            .filter_map(|image| image.first_sector.checked_mul(512)),
    );
    starts.sort_unstable();
    starts.dedup();
    starts
}

fn partitions(layout: &Layout) -> Vec<u64> {
    let block = u64::from(layout.logical_block_size);
    let gpt = layout
        .gpt
        .iter()
        .flat_map(|gpt| &gpt.entries)
        .filter(|entry| entry.type_guid.eq_ignore_ascii_case(ESP_GUID_TEXT))
        .map(|entry| entry.first_lba);
    let mbr = layout
        .mbr
        .iter()
        .filter(|mbr| mbr.signature_valid && !mbr.protective)
        .flat_map(|mbr| &mbr.entries)
        .filter(|entry| MBR_FAT.contains(&entry.kind))
        .map(|entry| u64::from(entry.start_lba));
    gpt.chain(mbr)
        .filter_map(|lba| lba.checked_mul(block))
        .collect()
}

#[cfg(test)]
mod tests;
