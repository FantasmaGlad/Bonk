//! Linux : `sysfs`, `mountinfo` et base udev pour l'énumération, `umount`, périphérique bloc en `O_EXCL`.

use std::io;
use std::path::Path;
use std::process::Command;

use bonk_core::{BlockDevice, BlockError};

use super::{Elevation, Platform, udisks};
use crate::disk::{Disk, Partition};
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::linux::{self, LinuxRoots};
use crate::node::BlockNode;
use crate::target::{OpenTargetError, mount_points};

/// Implémentation Linux.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Linux;

impl Platform for Linux {
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError> {
        linux::enumerate(&LinuxRoots::system())
    }

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
        linux::enumerate_recorded(&LinuxRoots::system())
    }

    fn enumerate_from_capture(&self, reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
        linux::enumerate_from_reads(reads)
    }

    fn unmount(&self, disk: &Disk) -> Result<Vec<String>, OpenTargetError> {
        unmount_all(disk)
    }

    fn prepare_access(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
    ) -> Result<(), BlockError> {
        if has_access(path, access) {
            return Ok(());
        }
        if elevation == Elevation::Never {
            return Err(denied(path));
        }
        // Lecture seule, quel que soit le besoin : c'est la même action polkit, mémorisée quelques
        // minutes, et fermer un descripteur en écriture ferait relire le disque par le noyau.
        udisks::open(path, "r", libc::O_CLOEXEC).map(drop)
    }

    fn open_disk(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
    ) -> Result<Box<dyn BlockDevice>, BlockError> {
        let node = match BlockNode::open(path, access) {
            Err(BlockError::Io(error))
                if error.kind() == io::ErrorKind::PermissionDenied
                    && elevation == Elevation::Auto =>
            {
                let (mode, flags) = match access {
                    Access::ReadOnly => ("r", libc::O_CLOEXEC),
                    Access::ReadWrite => ("rw", libc::O_CLOEXEC | libc::O_EXCL),
                };
                BlockNode::from_fd(udisks::open(path, mode, flags)?, path, access)
            }
            other => other,
        }?;
        Ok(Box::new(node))
    }

    fn is_privileged(&self) -> bool {
        // SAFETY : aucun argument, aucune erreur possible.
        unsafe { libc::geteuid() == 0 }
    }

    fn kernel_release(&self) -> Option<String> {
        std::fs::read_to_string("/proc/sys/kernel/osrelease")
            .ok()
            .map(|text| text.trim().to_owned())
    }
}

/// Démonte tout ce que porte le disque avec `umount` (outil fourni par le système : le démontage est
/// l'affaire de l'OS, pas de Bonk). Rend les points démontés.
///
/// # Errors
/// [`OpenTargetError::Unmount`] au premier démontage qui échoue ; les suivants ne sont pas tentés.
fn unmount_all(disk: &Disk) -> Result<Vec<String>, OpenTargetError> {
    let mounts = mount_points(disk);
    let error = match unmount_with(&mounts, &|mount| {
        Command::new("umount").arg("--").arg(mount).output()
    }) {
        Ok(done) => return Ok(done),
        Err(error) => error,
    };
    // `umount` n'a pas le droit de tout démonter pour un utilisateur ordinaire : le service, si.
    for partition in disk
        .partitions
        .iter()
        .filter(|p| !p.mount_points.is_empty())
    {
        if !still_mounted(partition) {
            continue;
        }
        if udisks::unmount(Path::new(&partition.path)).is_err() {
            return Err(error);
        }
    }
    Ok(mounts)
}

/// Un point de la partition est-il encore monté ? `umount` a pu en démonter une partie avant d'échouer.
fn still_mounted(partition: &Partition) -> bool {
    let table = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    table
        .lines()
        .any(|line| line.split_whitespace().next() == Some(partition.path.as_str()))
}

fn denied(path: &Path) -> BlockError {
    BlockError::Io(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{} : droits insuffisants, élévation désactivée",
            path.display()
        ),
    ))
}

/// Les droits de l'appelant suffisent-ils ? `access(2)` ne touche pas au disque.
fn has_access(path: &Path, access: Access) -> bool {
    use std::os::unix::ffi::OsStrExt;

    let Ok(name) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mode = match access {
        Access::ReadOnly => libc::R_OK,
        Access::ReadWrite => libc::R_OK | libc::W_OK,
    };
    // SAFETY : `name` est une chaîne C valide pendant tout l'appel.
    unsafe { libc::access(name.as_ptr(), mode) == 0 }
}

fn unmount_with(
    mounts: &[String],
    run: &dyn Fn(&str) -> io::Result<std::process::Output>,
) -> Result<Vec<String>, OpenTargetError> {
    let mut done = Vec::new();
    for mount in mounts {
        log::debug!(target: "bonk_hal::target", mount = mount.as_str(); "unmounting");
        match run(mount) {
            Ok(output) if output.status.success() => done.push(mount.clone()),
            Ok(output) => {
                return Err(OpenTargetError::Unmount(format!(
                    "démontage de {mount} impossible : {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            Err(err) => {
                return Err(OpenTargetError::Unmount(format!(
                    "démontage de {mount} impossible : {err}"
                )));
            }
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(code: i32, stderr: &str) -> io::Result<std::process::Output> {
        use std::os::unix::process::ExitStatusExt;
        Ok(std::process::Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    #[test]
    fn demontage_dans_l_ordre_et_arret_au_premier_echec() {
        use std::cell::RefCell;
        let called = RefCell::new(Vec::new());
        let ok = |mount: &str| {
            called.borrow_mut().push(mount.to_owned());
            output(0, "")
        };
        let mounts = vec!["/a/b".to_owned(), "/a".to_owned()];
        assert_eq!(unmount_with(&mounts, &ok).ok(), Some(mounts.clone()));
        assert_eq!(*called.borrow(), ["/a/b", "/a"]);

        let called = RefCell::new(Vec::new());
        let fail_on_first = |mount: &str| {
            called.borrow_mut().push(mount.to_owned());
            output(32, "umount: /a/b: target is busy.\n")
        };
        let error = unmount_with(&mounts, &fail_on_first).err();
        assert!(
            matches!(error, Some(OpenTargetError::Unmount(ref m)) if m.contains("busy") && m.contains("/a/b"))
        );
        // Le second point n'est pas tenté après l'échec du premier.
        assert_eq!(*called.borrow(), ["/a/b"]);
        // Un disque sans montage n'a rien à démonter.
        assert_eq!(unmount_with(&[], &ok).ok(), Some(vec![]));
    }

    #[test]
    fn un_outil_de_demontage_introuvable_est_une_erreur_claire() {
        let missing = |_: &str| {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "umount introuvable",
            ))
        };
        let error = unmount_with(&["/a".to_owned()], &missing).err();
        assert!(
            matches!(error, Some(OpenTargetError::Unmount(ref m)) if m.contains("introuvable"))
        );
    }
}
