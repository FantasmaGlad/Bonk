//! Essais sur une vraie clé : ce qui décide si l'écriture sans root est possible sur un téléphone.

use std::time::Instant;

use bonk_core::{BlockDevice, BlockError};
use sha2::{Digest, Sha256};

/// Loin du début : une image déjà écrite sur la clé reste intacte.
const BENCH_OFFSET: u64 = 16 << 30;
const BENCH_BYTES: usize = 64 << 20;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Empreinte d'une zone lue sur la clé, et débit.
///
/// # Errors
/// Lecture refusée ou clé débranchée.
pub fn read_hash(
    disk: &dyn BlockDevice,
    offset: u64,
    length: usize,
) -> Result<serde_json::Value, BlockError> {
    let mut data = vec![0; length];
    let started = Instant::now();
    disk.read_at(offset, &mut data)?;
    Ok(serde_json::json!({
        "sha256": hex(&Sha256::digest(&data)),
        "bytes": length,
        "megabytes_per_second": length as f64 / started.elapsed().as_secs_f64() / 1e6,
    }))
}

/// Suite pseudo-aléatoire : les données d'essai ne doivent pas être compressibles par la clé.
fn pattern(length: usize) -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut data = Vec::with_capacity(length + 8);
    while data.len() < length {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        data.extend_from_slice(&state.to_le_bytes());
    }
    data.truncate(length);
    data
}

fn write_and_check(disk: &mut dyn BlockDevice, data: &[u8]) -> Result<bool, BlockError> {
    disk.write_at(BENCH_OFFSET, data)?;
    disk.flush()?;
    let mut back = vec![0; data.len()];
    disk.read_at(BENCH_OFFSET, &mut back)?;
    Ok(back == data)
}

/// Écrit 64 Mio à 16 Gio, relit, puis rend à la zone son contenu et le vérifie. Même après un échec, la
/// restauration est tentée : son résultat est toujours dans le rapport.
///
/// # Errors
/// Clé trop petite, ou zone illisible avant tout changement : rien n'a alors été écrit.
pub fn write_bench(disk: &mut dyn BlockDevice) -> Result<serde_json::Value, BlockError> {
    if disk.size() < BENCH_OFFSET + BENCH_BYTES as u64 {
        return Err(BlockError::Io(std::io::Error::other(
            "drive too small for the write test",
        )));
    }
    let mut saved = vec![0; BENCH_BYTES];
    disk.read_at(BENCH_OFFSET, &mut saved)?;
    let data = pattern(BENCH_BYTES);
    let started = Instant::now();
    let written = write_and_check(disk, &data);
    let seconds = started.elapsed().as_secs_f64();
    let restored = write_and_check(disk, &saved);
    Ok(serde_json::json!({
        "written_and_read_back": written.as_ref().ok().copied().unwrap_or(false),
        "write_error": written.err().map(|error| error.to_string()),
        "restored": restored.as_ref().ok().copied().unwrap_or(false),
        "restore_error": restored.err().map(|error| error.to_string()),
        "write_and_read_seconds": seconds,
    }))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    struct Memory {
        data: RefCell<Vec<u8>>,
        size: u64,
        fail_writes: bool,
    }

    /// Une clé de 17 Gio dont seule la zone d'essai existe en mémoire.
    fn memory(fail_writes: bool) -> Memory {
        Memory {
            data: RefCell::new(vec![7; BENCH_BYTES]),
            size: BENCH_OFFSET + BENCH_BYTES as u64,
            fail_writes,
        }
    }

    impl BlockDevice for Memory {
        fn size(&self) -> u64 {
            self.size
        }
        fn block_size(&self) -> u32 {
            512
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            let start = usize::try_from(offset - BENCH_OFFSET).unwrap_or(0);
            buf.copy_from_slice(&self.data.borrow()[start..start + buf.len()]);
            Ok(())
        }
        fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
            if self.fail_writes {
                return Err(BlockError::Io(std::io::Error::other("refused")));
            }
            let start = usize::try_from(offset - BENCH_OFFSET).unwrap_or(0);
            self.data.borrow_mut()[start..start + data.len()].copy_from_slice(data);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }

    #[test]
    fn la_zone_retrouve_son_contenu() -> Result<(), BlockError> {
        let mut disk = memory(false);
        let report = write_bench(&mut disk)?;
        assert_eq!(report["written_and_read_back"], true);
        assert_eq!(report["restored"], true);
        assert!(disk.data.borrow().iter().all(|byte| *byte == 7));
        Ok(())
    }

    #[test]
    fn une_ecriture_refusee_est_rapportee() -> Result<(), BlockError> {
        let report = write_bench(&mut memory(true))?;
        assert_eq!(report["written_and_read_back"], false);
        assert!(report["write_error"].as_str().is_some());
        assert_eq!(report["restored"], false);
        Ok(())
    }

    #[test]
    fn une_cle_trop_petite_n_est_pas_touchee() {
        let mut disk = memory(false);
        disk.size = 1 << 30;
        assert!(write_bench(&mut disk).is_err());
    }

    #[test]
    fn empreinte_de_la_zone_lue() -> Result<(), BlockError> {
        let report = read_hash(&memory(false), BENCH_OFFSET, 512)?;
        assert_eq!(report["bytes"], 512);
        assert_eq!(report["sha256"].as_str().map(str::len), Some(64));
        Ok(())
    }
}
