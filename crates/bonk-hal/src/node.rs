//! Périphérique bloc réel ouvert pour lecture ou écriture brute (Linux).
//!
//! Complément de [`crate::image::FileDevice`], qui refuse tout ce qui n'est pas un fichier ordinaire :
//! `BlockNode` n'accepte, lui, **que** un disque entier (jamais une partition, jamais un fichier). Il
//! n'est construit qu'après les contrôles de sécurité de [`crate::target`] (filtre, identité,
//! démontage). À l'ouverture en écriture, `O_EXCL` fait refuser par le noyau un disque encore utilisé
//! (monté, ou ouvert par un autre programme) : dernier garde-fou, indépendant de nos propres contrôles.
//!
//! L'ouverture exige des droits : l'interface ne tourne jamais en administrateur, la ligne de commande
//! lancée par un administrateur le peut (D-03 : l'ouverture par le service du système viendra après).
//! Les tests n'ouvrent jamais de périphérique réel (invariant 3).

use std::path::{Path, PathBuf};

use bonk_core::{BlockDevice, BlockError};

use crate::image::Access;

/// Disque entier ouvert en accès brut.
#[derive(Debug)]
pub struct BlockNode {
    #[cfg(target_os = "linux")]
    file: std::fs::File,
    path: PathBuf,
    size: u64,
    block_size: u32,
    read_only: bool,
}

impl BlockNode {
    /// Chemin ouvert, pour les messages et le journal.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::fs::{self, OpenOptions};
    use std::io::{self, Seek, SeekFrom};
    use std::os::unix::fs::{FileExt, FileTypeExt, OpenOptionsExt};

    use super::*;
    use bonk_core::{check_access, check_geometry};

    fn invalid(message: &str) -> BlockError {
        BlockError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            message.to_owned(),
        ))
    }

    /// Taille de secteur logique du disque d'après le noyau ; 512 si elle n'est pas lisible.
    fn logical_block_size(name: &str) -> u32 {
        fs::read_to_string(format!("/sys/class/block/{name}/queue/logical_block_size"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
            .unwrap_or(512)
    }

    impl BlockNode {
        /// Ouvre un disque entier. `O_EXCL` en écriture : le noyau refuse un disque déjà utilisé.
        ///
        /// # Errors
        /// [`BlockError::Io`] : `InvalidInput` si le chemin n'est pas un disque entier (fichier,
        /// partition, périphérique caractère), `PermissionDenied` sans les droits, `EBUSY` si le disque
        /// est utilisé.
        pub fn open(path: &Path, access: Access) -> Result<Self, BlockError> {
            let resolved = fs::canonicalize(path)?;
            let metadata = fs::metadata(&resolved)?;
            if !metadata.file_type().is_block_device() {
                return Err(invalid("le chemin n'est pas un périphérique bloc"));
            }
            let name = resolved
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or_else(|| invalid("nom de périphérique illisible"))?
                .to_owned();
            // Une partition porte un fichier `partition` dans sysfs : on n'écrit que des disques entiers.
            if Path::new(&format!("/sys/class/block/{name}/partition")).exists() {
                return Err(invalid("le chemin est une partition, pas un disque entier"));
            }
            let read_only = access == Access::ReadOnly;
            let mut options = OpenOptions::new();
            options.read(true).write(!read_only);
            if !read_only {
                options.custom_flags(libc::O_EXCL);
            }
            let mut file = options.open(&resolved)?;
            // La taille d'un périphérique bloc ne se lit pas dans ses métadonnées.
            let size = file.seek(SeekFrom::End(0))?;
            let block_size = logical_block_size(&name);
            check_geometry(size, block_size)?;
            log::debug!(
                target: "bonk_hal::node",
                path = resolved.display().to_string().as_str(), size_bytes = size,
                block_size = block_size, read_only = read_only;
                "block device opened"
            );
            Ok(Self {
                file,
                path: resolved,
                size,
                block_size,
                read_only,
            })
        }
    }

    impl BlockDevice for BlockNode {
        fn size(&self) -> u64 {
            self.size
        }

        fn block_size(&self) -> u32 {
            self.block_size
        }

        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            check_access(self.size, self.block_size, offset, buf.len())?;
            self.file.read_exact_at(buf, offset)?;
            Ok(())
        }

        fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
            if self.read_only {
                return Err(BlockError::ReadOnly);
            }
            check_access(self.size, self.block_size, offset, data.len())?;
            self.file.write_all_at(data, offset)?;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            if !self.read_only {
                self.file.sync_all()?;
            }
            Ok(())
        }
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::*;

    impl BlockNode {
        /// L'accès brut aux disques n'est pas encore disponible hors Linux.
        ///
        /// # Errors
        /// Toujours [`BlockError::Io`] (`Unsupported`).
        pub fn open(_path: &Path, _access: Access) -> Result<Self, BlockError> {
            Err(BlockError::Io(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "accès brut aux disques non disponible sur ce système",
            )))
        }
    }

    impl BlockDevice for BlockNode {
        fn size(&self) -> u64 {
            self.size
        }

        fn block_size(&self) -> u32 {
            self.block_size
        }

        fn read_at(&self, _offset: u64, _buf: &mut [u8]) -> Result<(), BlockError> {
            Err(BlockError::ReadOnly)
        }

        fn write_at(&mut self, _offset: u64, _data: &[u8]) -> Result<(), BlockError> {
            Err(BlockError::ReadOnly)
        }

        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Aucun test n'ouvre de vrai disque : on vérifie seulement ce que `BlockNode` refuse.
    #[cfg(target_os = "linux")]
    #[test]
    fn refuse_tout_ce_qui_n_est_pas_un_disque_entier() {
        let kind = |path: &str| match BlockNode::open(Path::new(path), Access::ReadOnly) {
            Err(BlockError::Io(err)) => Some(err.kind()),
            _ => None,
        };
        // Un fichier ordinaire, un répertoire et un périphérique caractère ne sont pas des disques.
        assert_eq!(
            kind("/proc/self/mountinfo"),
            Some(std::io::ErrorKind::InvalidInput)
        );
        assert_eq!(kind("/proc"), Some(std::io::ErrorKind::InvalidInput));
        assert_eq!(kind("/dev/null"), Some(std::io::ErrorKind::InvalidInput));
        // Un chemin absent est une erreur d'entrée-sortie, pas une ouverture.
        assert_eq!(
            kind("/dev/disque-inexistant"),
            Some(std::io::ErrorKind::NotFound)
        );
    }
}
