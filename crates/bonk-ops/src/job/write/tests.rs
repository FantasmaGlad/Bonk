use bonk_core::BlockDevice;
use bonk_core::partition::inspect;
use bonk_core::repair::needs_repair;

use crate::errors::Code;
use bonk_core::format::table::{TableKind, write_table};
use bonk_core::format::{Filesystem, Ids};
use bonk_core::{BlockError, MemoryDevice};

use super::*;

/// A GPT image smaller than the drive: the backup GPT is missing at the end.
fn image_sur_un_plus_grand_support() -> Result<MemoryDevice, Box<dyn std::error::Error>> {
    let mut image = MemoryDevice::new(8 << 20, 512)?;
    write_table(
        &mut image,
        TableKind::Gpt,
        Filesystem::Fat32,
        None,
        &Ids::from_seed(b"gpt"),
        false,
    )?;
    let mut support = MemoryDevice::new(16 << 20, 512)?;
    support.write_at(0, image.as_bytes())?;
    Ok(support)
}

/// Accepts writes past a point without keeping them, like a failing card.
struct PerdLaFin {
    inner: MemoryDevice,
    limite: u64,
}

impl BlockDevice for PerdLaFin {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn block_size(&self) -> u32 {
        self.inner.block_size()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.inner.read_at(offset, buf)
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        if offset >= self.limite {
            return Ok(());
        }
        self.inner.write_at(offset, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

#[test]
fn la_gpt_reparee_est_relue() -> Result<(), Box<dyn std::error::Error>> {
    let mut support = image_sur_un_plus_grand_support()?;
    assert!(needs_repair(&inspect(&support)?));
    assert!(
        repair_if_needed(&mut support)
            .map_err(|e| e.message)?
            .is_some()
    );
    assert!(!needs_repair(&inspect(&support)?));
    Ok(())
}

#[test]
fn une_reparation_que_le_support_ne_garde_pas_est_un_echec()
-> Result<(), Box<dyn std::error::Error>> {
    let mut support = PerdLaFin {
        inner: image_sur_un_plus_grand_support()?,
        limite: 8 << 20,
    };
    let erreur = repair_if_needed(&mut support)
        .err()
        .ok_or("réparation acceptée")?;
    assert_eq!(erreur.code, Code::GptRepairFailed);
    Ok(())
}
