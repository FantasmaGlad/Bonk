//! `partition::inspect` sur un disque arbitraire : une image reçue d'ailleurs ne doit jamais le faire
//! paniquer ni l'amener à allouer sans borne, quelle que soit sa table de partitions.
#![no_main]

use bonk_core::MemoryDevice;
use bonk_core::partition::{crc32, inspect};
use libfuzzer_sys::fuzz_target;

const SECTOR: usize = 512;

fn le_u32(buf: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([buf[at], buf[at + 1], buf[at + 2], buf[at + 3]])
}

/// Recalcule les CRC du GPT primaire : sans cela, le fuzzeur s'arrête presque toujours à leur contrôle
/// et n'atteint pas l'analyse des entrées.
fn fix_primary_gpt_crcs(disk: &mut [u8]) {
    let header = SECTOR;
    if disk.len() < 2 * SECTOR {
        return;
    }
    let lba = u64::from_le_bytes(disk[header + 72..header + 80].try_into().unwrap_or([0xFF; 8]));
    let count = u64::from(le_u32(disk, header + 80));
    let entry_size = u64::from(le_u32(disk, header + 84));
    let range = lba
        .checked_mul(SECTOR as u64)
        .zip(count.checked_mul(entry_size))
        .and_then(|(start, len)| Some((usize::try_from(start).ok()?, usize::try_from(len).ok()?)))
        .and_then(|(start, len)| Some(start..start.checked_add(len)?));
    if let Some(range) = range.filter(|r| r.end <= disk.len()) {
        let crc = crc32(&disk[range]);
        disk[header + 88..header + 92].copy_from_slice(&crc.to_le_bytes());
    }
    let size = le_u32(disk, header + 12) as usize;
    if (92..=SECTOR).contains(&size) {
        disk[header + 16..header + 20].fill(0);
        let crc = crc32(&disk[header..header + size]);
        disk[header + 16..header + 20].copy_from_slice(&crc.to_le_bytes());
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else {
        return;
    };
    let mut disk = rest[..rest.len() - rest.len() % SECTOR].to_vec();
    if mode & 1 == 1 {
        fix_primary_gpt_crcs(&mut disk);
    }
    if let Ok(dev) = MemoryDevice::from_bytes(disk, SECTOR as u32) {
        // Un contenu incohérent devient un constat ou une erreur : seule une panique est un défaut.
        let _ = inspect(&dev);
    }
});
