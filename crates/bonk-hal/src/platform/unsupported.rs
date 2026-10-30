//! Système sans accès aux disques : chaque opération échoue clairement, jamais en silence. C'est aussi
//! ce qui permet de lancer la ligne de commande partout, et d'y tester les chemins d'erreur.

use std::path::Path;

use bonk_core::{BlockDevice, BlockError};

use super::{Elevation, Platform};
use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::target::OpenTargetError;

/// Implémentation pour un système que Bonk ne sait pas encore piloter.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unsupported;

fn no_raw_access() -> BlockError {
    BlockError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "accès brut aux disques non disponible sur ce système",
    ))
}

fn unsupported() -> EnumerateError {
    EnumerateError::Unsupported {
        os: std::env::consts::OS,
    }
}

impl Platform for Unsupported {
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError> {
        Err(unsupported())
    }

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
        (Err(unsupported()), Vec::new())
    }

    fn enumerate_from_capture(&self, _reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
        Err(unsupported())
    }

    fn unmount(&self, _disk: &Disk) -> Result<Vec<String>, OpenTargetError> {
        Err(OpenTargetError::Unmount(format!(
            "démontage non disponible sous {}",
            std::env::consts::OS
        )))
    }

    fn prepare_access(
        &self,
        _path: &Path,
        _access: Access,
        _elevation: Elevation,
    ) -> Result<(), BlockError> {
        Err(no_raw_access())
    }

    fn open_disk(
        &self,
        _path: &Path,
        _access: Access,
        _elevation: Elevation,
    ) -> Result<Box<dyn BlockDevice>, BlockError> {
        Err(no_raw_access())
    }

    fn is_privileged(&self) -> bool {
        false
    }

    fn kernel_release(&self) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_operation_echoue_sans_rien_pretendre() {
        let platform = Unsupported;
        assert!(matches!(
            platform.enumerate(),
            Err(EnumerateError::Unsupported { .. })
        ));
        let (result, reads) = platform.enumerate_recorded();
        assert!(result.is_err());
        assert!(reads.is_empty());
        assert!(platform.enumerate_from_capture(&[]).is_err());
        assert!(matches!(
            platform.open_disk(Path::new("disque"), Access::ReadOnly, Elevation::Auto),
            Err(BlockError::Io(err)) if err.kind() == std::io::ErrorKind::Unsupported
        ));
        assert_eq!(platform.kernel_release(), None);
    }
}
