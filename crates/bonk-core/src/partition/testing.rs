use super::*;
use crate::bytes::{put_u32, put_u64};

pub use crate::format::table::{GUID_ESP as ESP, GUID_LINUX as LINUX};

/// Valid GPT image of `sectors` 512-byte sectors with `(start, end, type, name)` partitions.
pub fn gpt_image(sectors: u64, parts: &[(u64, u64, [u8; 16], &str)]) -> Vec<u8> {
    let bs = 512usize;
    let mut img = vec![0u8; usize::try_from(sectors).unwrap_or(0) * bs];
    img[446 + 4] = MBR_PROTECTIVE;
    put_u32(&mut img, 446 + 8, 1);
    put_u32(
        &mut img,
        446 + 12,
        u32::try_from(sectors - 1).unwrap_or(u32::MAX),
    );
    img[510] = 0x55;
    img[511] = 0xAA;
    let mut table = vec![0u8; 128 * 128];
    for (i, (first, last, kind, name)) in parts.iter().enumerate() {
        let e = &mut table[i * 128..(i + 1) * 128];
        e[..16].copy_from_slice(kind);
        e[16..32].copy_from_slice(&[i as u8 + 1; 16]);
        put_u64(e, 32, *first);
        put_u64(e, 40, *last);
        for (j, unit) in name.encode_utf16().enumerate() {
            e[56 + j * 2..58 + j * 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    let table_crc = crc32(&table);
    let total = usize::try_from(sectors).unwrap_or(0);
    let write_header = |img: &mut Vec<u8>, at: usize, my: u64, alt: u64, entries_lba: u64| {
        let h = &mut img[at * bs..at * bs + bs];
        h[..8].copy_from_slice(GPT_SIGNATURE);
        put_u32(h, 8, 0x0001_0000);
        put_u32(h, 12, 92);
        put_u64(h, 24, my);
        put_u64(h, 32, alt);
        put_u64(h, 40, 34);
        put_u64(h, 48, sectors - 34);
        h[56..72].copy_from_slice(&[0xD1; 16]);
        put_u64(h, 72, entries_lba);
        put_u32(h, 80, 128);
        put_u32(h, 84, 128);
        put_u32(h, 88, table_crc);
        let crc = crc32(&h[..92]);
        put_u32(h, 16, crc);
    };
    write_header(&mut img, 1, 1, sectors - 1, 2);
    img[2 * bs..2 * bs + table.len()].copy_from_slice(&table);
    let backup_table = total - 33;
    img[backup_table * bs..backup_table * bs + table.len()].copy_from_slice(&table);
    write_header(&mut img, total - 1, sectors - 1, 1, sectors - 33);
    img
}
