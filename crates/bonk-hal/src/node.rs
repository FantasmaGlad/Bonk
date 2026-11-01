//! Périphérique bloc réel ouvert pour lecture ou écriture brute (Linux).
//!
//! Complément de [`crate::image::FileDevice`], qui refuse tout ce qui n'est pas un fichier ordinaire :
//! `BlockNode` n'accepte, lui, **que** un disque entier (jamais une partition, jamais un fichier). Il
//! n'est construit qu'après les contrôles de sécurité de [`crate::target`] (filtre, identité,
//! démontage). À l'ouverture en écriture, `O_EXCL` fait refuser par le noyau un disque encore utilisé
//! (monté, ou ouvert par un autre programme) : dernier garde-fou, indépendant de nos propres contrôles.
//!
//! L'ouverture exige des droits : ceux de l'appelant, ou un descripteur remis par le service du
//! système ([`BlockNode::from_fd`]) ; l'interface ne tourne jamais en administrateur. Les tests
//! n'ouvrent jamais de périphérique réel.

use std::path::{Path, PathBuf};

use bonk_core::{BlockDevice, BlockError};

use crate::image::Access;

/// Disque entier ouvert en accès brut.
#[derive(Debug)]
pub struct BlockNode {
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

mod imp {
    use std::fs::{self, OpenOptions};
    use std::io::{self, Seek, SeekFrom};
    use std::os::fd::OwnedFd;
    use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt};

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

    /// Le nom noyau du périphérique, s'il s'agit d'un disque entier : jamais une partition ni un fichier.
    fn whole_disk_name(resolved: &Path) -> Result<String, BlockError> {
        if !fs::metadata(resolved)?.file_type().is_block_device() {
            return Err(invalid("the path is not a block device"));
        }
        let name = resolved
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("unreadable device name"))?
            .to_owned();
        // Une partition porte un fichier `partition` dans sysfs : on n'écrit que des disques entiers.
        if Path::new(&format!("/sys/class/block/{name}/partition")).exists() {
            return Err(invalid("the path is a partition, not a whole disk"));
        }
        Ok(name)
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
            let name = whole_disk_name(&resolved)?;
            let read_only = access == Access::ReadOnly;
            let mut options = OpenOptions::new();
            options.read(true).write(!read_only);
            if !read_only {
                options.custom_flags(libc::O_EXCL);
            }
            Self::finish(options.open(&resolved)?, resolved, name, read_only)
        }

        /// Reprend un descripteur ouvert par le service du système. Il n'est cru sur rien : c'est bien un
        /// disque entier, et le même périphérique que `path`. Ne prouve pas que ce soit celui qui a été
        /// choisi : `path` a pu changer de disque après la revérification.
        ///
        /// # Errors
        /// [`BlockError::Io`] : `InvalidInput` si le descripteur n'est pas ce disque.
        pub fn from_fd(fd: OwnedFd, path: &Path, access: Access) -> Result<Self, BlockError> {
            let resolved = fs::canonicalize(path)?;
            let name = whole_disk_name(&resolved)?;
            let file = fs::File::from(fd);
            let (held, expected) = (file.metadata()?, fs::metadata(&resolved)?);
            if !held.file_type().is_block_device() || held.rdev() != expected.rdev() {
                return Err(invalid("the received descriptor is not this disk"));
            }
            Self::finish(file, resolved, name, access == Access::ReadOnly)
        }

        fn finish(
            mut file: fs::File,
            resolved: PathBuf,
            name: String,
            read_only: bool,
        ) -> Result<Self, BlockError> {
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

        /// `posix_fadvise(DONTNEED)` sur tout le périphérique : le noyau abandonne ses pages propres
        /// (celles que `flush` vient d'écrire), donc la relecture qui suit lit le support. Constaté sur
        /// une clé USB 2.0 : sans cela, la relecture de contrôle tournait à 2 193 Mo/s, c'est-à-dire
        /// dans le cache. Ne demande aucun privilège, contrairement à `BLKFLSBUF`.
        fn drop_cache(&mut self) -> Result<(), BlockError> {
            use std::os::fd::AsRawFd;

            // SAFETY : le descripteur appartient à `self.file`, ouvert pour toute la durée de l'appel.
            let code = unsafe {
                libc::posix_fadvise(self.file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED)
            };
            if code == 0 {
                Ok(())
            } else {
                Err(BlockError::Io(std::io::Error::from_raw_os_error(code)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::OwnedFd;

    use super::*;

    /// Un descripteur qui n'est pas un disque est refusé, quoi qu'en dise le chemin.
    #[test]
    fn un_descripteur_recu_n_est_pas_cru_sur_parole() {
        let fd = |path: &str| -> OwnedFd {
            std::fs::File::open(path)
                .map(OwnedFd::from)
                .unwrap_or_else(|e| panic!("{path} : {e}"))
        };
        for path in ["/dev/null", "/proc/self/mountinfo"] {
            let refus = BlockNode::from_fd(fd(path), Path::new(path), Access::ReadOnly);
            assert!(
                matches!(refus, Err(BlockError::Io(ref e)) if e.kind() == std::io::ErrorKind::InvalidInput),
                "{path}"
            );
        }
    }

    /// Aucun test n'ouvre de vrai disque : on vérifie seulement ce que `BlockNode` refuse.
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
