use std::path::{Path, PathBuf};

use bonk_core::{BlockDevice, BlockError};

use crate::image::Access;

#[derive(Debug)]
pub struct BlockNode {
    file: std::fs::File,
    path: PathBuf,
    size: u64,
    block_size: u32,
    read_only: bool,
}

impl BlockNode {
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
    use crate::linux::LinuxRoots;
    use bonk_core::{check_access, check_geometry};

    fn invalid(message: &str) -> BlockError {
        BlockError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            message.to_owned(),
        ))
    }

    /// 512 when unreadable.
    fn logical_block_size(roots: &LinuxRoots, name: &str) -> u32 {
        fs::read_to_string(
            roots
                .sys
                .join(format!("class/block/{name}/queue/logical_block_size")),
        )
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(512)
    }

    fn is_partition(roots: &LinuxRoots, name: &str) -> bool {
        roots
            .sys
            .join(format!("class/block/{name}/partition"))
            .exists()
    }

    /// Kernel name, for a whole disk only: never a partition or a file.
    fn whole_disk_name(roots: &LinuxRoots, resolved: &Path) -> Result<String, BlockError> {
        if !fs::metadata(resolved)?.file_type().is_block_device() {
            return Err(invalid("the path is not a block device"));
        }
        let name = resolved
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| invalid("unreadable device name"))?
            .to_owned();
        // A partition has a `partition` file in sysfs: only whole disks are written.
        if is_partition(roots, &name) {
            return Err(invalid("the path is a partition, not a whole disk"));
        }
        Ok(name)
    }

    impl BlockNode {
        /// `O_EXCL` for writing: the kernel refuses a disk in use.
        ///
        /// # Errors
        /// [`BlockError::Io`]: `InvalidInput` if the path is not a whole disk (file, partition, character
        /// device), `PermissionDenied` without rights, `EBUSY` if the disk is in use.
        pub fn open(path: &Path, access: Access) -> Result<Self, BlockError> {
            let resolved = fs::canonicalize(path)?;
            let name = whole_disk_name(&LinuxRoots::system(), &resolved)?;
            let read_only = access == Access::ReadOnly;
            let mut options = OpenOptions::new();
            options.read(true).write(!read_only);
            if !read_only {
                options.custom_flags(libc::O_EXCL);
            }
            Self::finish(options.open(&resolved)?, resolved, name, read_only)
        }

        /// Takes a descriptor opened by the system service and trusts nothing: it must be a whole disk and
        /// the same device as `path`. It does not prove it is the chosen one: `path` may point to another
        /// disk after the re-check.
        ///
        /// # Errors
        /// [`BlockError::Io`]: `InvalidInput` if the descriptor is not that disk.
        pub(crate) fn from_fd(
            fd: OwnedFd,
            path: &Path,
            access: Access,
        ) -> Result<Self, BlockError> {
            let resolved = fs::canonicalize(path)?;
            let name = whole_disk_name(&LinuxRoots::system(), &resolved)?;
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
            // A block device size is not in its metadata.
            let size = file.seek(SeekFrom::End(0))?;
            let block_size = logical_block_size(&LinuxRoots::system(), &name);
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

        /// `posix_fadvise(DONTNEED)` on the whole device: the kernel drops its clean pages, so the read-back
        /// reads the drive. Seen on a USB 2.0 key: without it the read-back ran at 2,193 MB/s, from cache.
        /// Needs no privilege, unlike `BLKFLSBUF`.
        fn drop_cache(&mut self) -> Result<(), BlockError> {
            use std::os::fd::AsRawFd;

            // SAFETY: the descriptor belongs to `self.file`, open for the whole call.
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn taille_de_secteur_et_partition_lues_sous_la_racine_donnee() -> io::Result<()> {
            let racine = std::env::temp_dir().join(format!("bonk-node-{}", std::process::id()));
            let disque = racine.join("class/block/sdz");
            fs::create_dir_all(disque.join("queue"))?;
            fs::create_dir_all(racine.join("class/block/sdz1"))?;
            fs::write(disque.join("queue/logical_block_size"), "4096\n")?;
            fs::write(racine.join("class/block/sdz1/partition"), "1\n")?;
            let roots = LinuxRoots {
                sys: racine.clone(),
                ..LinuxRoots::system()
            };
            assert_eq!(logical_block_size(&roots, "sdz"), 4096);
            assert_eq!(logical_block_size(&roots, "absent"), 512);
            assert!(is_partition(&roots, "sdz1"));
            assert!(!is_partition(&roots, "sdz"));
            fs::remove_dir_all(&racine)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::fd::OwnedFd;

    use super::*;

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

    /// No test opens a real disk: only what `BlockNode` refuses is checked.
    #[test]
    fn refuse_tout_ce_qui_n_est_pas_un_disque_entier() {
        let kind = |path: &str| match BlockNode::open(Path::new(path), Access::ReadOnly) {
            Err(BlockError::Io(err)) => Some(err.kind()),
            _ => None,
        };
        assert_eq!(
            kind("/proc/self/mountinfo"),
            Some(std::io::ErrorKind::InvalidInput)
        );
        assert_eq!(kind("/proc"), Some(std::io::ErrorKind::InvalidInput));
        assert_eq!(kind("/dev/null"), Some(std::io::ErrorKind::InvalidInput));
        assert_eq!(
            kind("/dev/disque-inexistant"),
            Some(std::io::ErrorKind::NotFound)
        );
    }
}
