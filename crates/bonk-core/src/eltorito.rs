const ISO_SECTOR: usize = 2048;
const BOOT_RECORD: usize = 17 * ISO_SECTOR;
const ENTRY: usize = 32;
const BOOTABLE: u8 = 0x88;
const SECTION_MORE: u8 = 0x90;
const SECTION_LAST: u8 = 0x91;
/// Platform identifier of an EFI boot image.
pub const PLATFORM_EFI: u8 = 0xEF;

/// A bootable entry: where its image lies, in 512-byte sectors from the start of the ISO.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootImage {
    pub platform: u8,
    pub first_sector: u64,
    pub sectors: u64,
}

// Boot record volume descriptor, El Torito Bootable CD-ROM Format Specification 1.0, section 2.0.
fn catalog_lba(head: &[u8]) -> Option<usize> {
    let record = head.get(BOOT_RECORD..BOOT_RECORD + ISO_SECTOR)?;
    let is_el_torito = record[0] == 0
        && &record[1..6] == b"CD001"
        && record[7..].starts_with(b"EL TORITO SPECIFICATION");
    is_el_torito.then(|| {
        u32::from_le_bytes([record[0x47], record[0x48], record[0x49], record[0x4A]]) as usize
    })
}

fn entry(bytes: &[u8], platform: u8) -> Option<BootImage> {
    let sectors = u64::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let rba = u64::from(u32::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11],
    ]));
    (bytes[0] == BOOTABLE).then_some(BootImage {
        platform,
        first_sector: rba * (ISO_SECTOR as u64 / 512),
        sectors,
    })
}

/// The bootable entries the catalog lists, `[]` without a catalog in `head`.
#[must_use]
pub fn boot_images(head: &[u8]) -> Vec<BootImage> {
    let Some(catalog) =
        catalog_lba(head).and_then(|lba| head.get(lba * ISO_SECTOR..(lba + 1) * ISO_SECTOR))
    else {
        return Vec::new();
    };
    let (entries, _) = catalog.as_chunks::<ENTRY>();
    let mut entries = entries.iter();
    let Some(validation) = entries
        .next()
        .filter(|v| v[0] == 1 && v[30] == 0x55 && v[31] == 0xAA)
    else {
        return Vec::new();
    };
    let mut found: Vec<BootImage> = entries
        .next()
        .and_then(|e| entry(e, validation[1]))
        .into_iter()
        .collect();
    while let Some(header) = entries
        .next()
        .filter(|h| matches!(h[0], SECTION_MORE | SECTION_LAST))
    {
        let count = usize::from(u16::from_le_bytes([header[2], header[3]]));
        found.extend(
            entries
                .by_ref()
                .take(count)
                .filter_map(|e| entry(e, header[1])),
        );
        if header[0] == SECTION_LAST {
            break;
        }
    }
    found
}
