//! Windows : l'implémentation de `Platform`. La lecture des faits est dans `windows_query`, l'accès brut
//! dans `windows_disk`.

use std::mem::size_of;
use std::path::Path;
use std::ptr::null_mut;
use std::time::Instant;

use bonk_core::format::Filesystem;
use bonk_core::{BlockDevice, BlockError};

use super::{Capability, Elevation, Platform, native_capabilities, windows_disk, windows_helper};
use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::target::OpenTargetError;
use crate::windows_facts::{SOURCE, normalize};

use super::windows_query::{Handle, collect};

#[derive(Debug, Clone, Copy, Default)]
pub struct Windows;

impl Platform for Windows {
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError> {
        normalize(&collect()?)
    }

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
        let started = Instant::now();
        let facts = collect();
        let micros = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
        match facts {
            Ok(facts) => {
                let content = serde_json::to_string(&facts).unwrap_or_default();
                let read = SourceRead {
                    path: SOURCE.to_owned(),
                    kind: "file".to_owned(),
                    ok: true,
                    bytes: Some(content.len()),
                    error: None,
                    micros,
                    content: Some(content),
                };
                (normalize(&facts), vec![read])
            }
            Err(error) => {
                let read = SourceRead {
                    path: SOURCE.to_owned(),
                    kind: "file".to_owned(),
                    ok: false,
                    bytes: None,
                    error: Some(error.to_string()),
                    micros,
                    content: None,
                };
                (Err(error), vec![read])
            }
        }
    }

    fn enumerate_from_capture(&self, _reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
        Err(EnumerateError::Malformed {
            what: "capture".to_owned(),
            detail: "no Windows facts in this capture".to_owned(),
        })
    }

    fn unmount(&self, disk: &Disk, elevation: Elevation) -> Result<Vec<String>, OpenTargetError> {
        if through_helper(elevation) {
            windows_helper::unmount(disk)
        } else {
            windows_disk::unmount(&disk.path)
        }
    }

    fn prepare_access(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<(), BlockError> {
        if through_helper(elevation) {
            windows_helper::prepare(path, access)
        } else {
            windows_disk::prepare(path, access)
        }
    }

    fn open_disk(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn BlockDevice>, BlockError> {
        if through_helper(elevation) {
            windows_helper::open(path, access)
        } else {
            Ok(Box::new(windows_disk::open(path, access)?))
        }
    }

    fn capabilities(&self) -> Vec<Capability> {
        let mut all = native_capabilities(Capability::yes("elevate"));
        for capability in &mut all {
            // exFAT et NTFS passent par `Format-Volume`, présent sur tout Windows pris en charge.
            if ["format.exfat", "format.ntfs"].contains(&capability.name.as_str()) {
                *capability = Capability::yes(&capability.name);
            }
        }
        all
    }

    fn format_partition(
        &self,
        partition: &Path,
        fs: Filesystem,
        label: Option<&str>,
        elevation: Elevation,
    ) -> Result<(), BlockError> {
        if through_helper(elevation) {
            windows_helper::format(partition, fs, label)
        } else {
            windows_disk::format_partition(partition, fs, label)
        }
    }

    fn rescan(&self, disk: &Path, elevation: Elevation) -> Result<(), BlockError> {
        if through_helper(elevation) {
            windows_helper::rescan(disk)
        } else {
            windows_disk::rescan(disk)
        }
    }

    fn is_privileged(&self) -> bool {
        is_elevated()
    }

    fn kernel_release(&self) -> Option<String> {
        None
    }
}

/// Passer par le helper ? Sans droits et si l'élévation est permise. `BONK_WINDOWS_HELPER=always` l'impose à
/// un processus déjà élevé : un runner de CI n'a aucun moyen de lancer Bonk sans droits, et le chemin complet
/// doit y être essayé.
fn through_helper(elevation: Elevation) -> bool {
    elevation == Elevation::Auto
        && (!is_elevated()
            || std::env::var_os("BONK_WINDOWS_HELPER").is_some_and(|v| v == "always"))
}

/// Le processus tourne-t-il élevé ? Illisible : non, la fenêtre s'ouvre et l'écriture sera refusée.
fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = null_mut();
    // SAFETY : pseudo-poignée du processus ; `token` reçoit une poignée fermée par `Handle`.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
        return false;
    }
    let token = Handle(token);
    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned = 0u32;
    // SAFETY : sortie valide pour la taille annoncée.
    let done = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&raw mut elevation).cast(),
            u32::try_from(size_of::<TOKEN_ELEVATION>()).unwrap_or(0),
            &raw mut returned,
        )
    };
    done != 0 && elevation.TokenIsElevated != 0
}
