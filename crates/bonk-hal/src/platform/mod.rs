#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_disk;
#[cfg(target_os = "linux")]
mod udisks;
mod unsupported;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
mod windows_disk;
#[cfg(windows)]
mod windows_helper;
#[cfg(windows)]
mod windows_query;

use std::path::{Path, PathBuf};

use bonk_core::format::Filesystem;
use bonk_core::{BlockDevice, BlockError};
use schemars::JsonSchema;
use serde::Serialize;

pub use unsupported::Unsupported;

use crate::disk::Disk;
use crate::enumerate::{EnumerateError, SourceRead};
use crate::image::Access;
use crate::target::OpenTargetError;

/// Whether to ask the system for missing rights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Elevation {
    #[default]
    Auto,
    /// Caller's rights only: for a service with nobody in front of the screen.
    Never,
}

/// Why a function is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// A host tool is missing: `detail` names it.
    MissingTool,
    /// This system does not allow it, or Bonk does not know how yet.
    OsUnsupported,
    /// The system refuses this user.
    NoPermission,
}

/// A function the host allows, or why it does not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Capability {
    /// Stable name: `write`, `format.fat32`...
    pub name: String,
    pub available: bool,
    pub missing: Option<Missing>,
    pub detail: Option<String>,
}

impl Capability {
    #[must_use]
    pub fn yes(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            available: true,
            missing: None,
            detail: None,
        }
    }

    #[must_use]
    pub fn no(name: &str, missing: Missing, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            available: false,
            missing: Some(missing),
            detail: Some(detail.into()),
        }
    }
}

#[must_use]
pub fn format_capability(fs: Filesystem) -> String {
    format!("format.{}", fs.name())
}

pub trait Platform: Send + Sync {
    /// Raw facts, judged afterwards by [`crate::safety::evaluate`].
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError>;

    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>);

    fn enumerate_from_capture(&self, reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError>;

    /// Returns the unmounted points; stops at the first failure. With [`Elevation::Never`] no authorization
    /// is asked.
    fn unmount(&self, disk: &Disk, elevation: Elevation) -> Result<Vec<String>, OpenTargetError>;

    /// Gets the right to open the disk without keeping it open. Called before any unmount: a refusal must
    /// leave no trace on the key.
    fn prepare_access(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), BlockError>;

    /// Only called after the `bonk-ops` guards. `cancelled` gives up waiting for an authorization
    /// (`ErrorKind::Interrupted`).
    fn open_disk(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn BlockDevice>, BlockError>;

    fn capabilities(&self) -> Vec<Capability>;

    /// Host tools (NTFS, ext4). Only the tool's result counts; its output is not parsed.
    fn format_partition(
        &self,
        partition: &Path,
        fs: Filesystem,
        label: Option<&str>,
        elevation: Elevation,
    ) -> Result<(), BlockError>;

    /// Asks the system to reread the partition table just replaced: otherwise it may show the old partitions,
    /// or none, for a while.
    fn rescan(&self, disk: &Path, elevation: Elevation) -> Result<(), BlockError>;

    /// The window refuses to open as administrator.
    fn is_privileged(&self) -> bool;

    fn kernel_release(&self) -> Option<String>;
}

/// From its shape: `debug layout` must say so even when the path does not exist.
#[must_use]
pub fn is_device_path(path: &str) -> bool {
    path.starts_with("/dev/") || path.starts_with(r"\\.\")
}

/// `bonk elevated-helper`, Windows only.
///
/// # Errors
/// Unusable pipe, or a client that is not the announced one; always elsewhere.
pub fn run_elevated_helper(client: u32, pipe: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        windows_helper::serve(client, pipe)
    }
    #[cfg(not(windows))]
    {
        let _ = (client, pipe);
        Err("the elevated helper only exists on Windows".to_owned())
    }
}

/// What every writing host reports; only elevation and host-written filesystems vary.
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn writing_capabilities(
    elevate: Capability,
    hosted_format: impl Fn(Filesystem, &str) -> Capability,
) -> Vec<Capability> {
    let mut all = vec![
        Capability::yes("write"),
        elevate,
        Capability::no("eject", Missing::OsUnsupported, "not in Bonk yet"),
        Capability::yes("cancel"),
    ];
    all.extend(Filesystem::ALL.map(|fs| {
        let name = format_capability(fs);
        if fs.is_native() {
            Capability::yes(&name)
        } else {
            hosted_format(fs, &name)
        }
    }));
    all
}

#[cfg(any(windows, target_os = "macos"))]
fn native_capabilities(elevate: Capability) -> Vec<Capability> {
    writing_capabilities(elevate, |_, name| {
        Capability::no(
            name,
            Missing::OsUnsupported,
            "not in Bonk yet on this system",
        )
    })
}

#[must_use]
pub fn current() -> &'static dyn Platform {
    #[cfg(target_os = "linux")]
    {
        &linux::Linux
    }
    #[cfg(windows)]
    {
        &windows::Windows
    }
    #[cfg(target_os = "macos")]
    {
        &macos::Macos
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        &Unsupported
    }
}

#[must_use]
pub fn config_file(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    config_file_for(std::env::consts::OS, var)
}

#[must_use]
pub fn cache_dir(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    cache_dir_for(std::env::consts::OS, var)
}

fn cache_dir_for(os: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    match os {
        "windows" => var("LOCALAPPDATA").map(|dir| PathBuf::from(dir).join("Bonk").join("cache")),
        "macos" => var("HOME").map(|home| PathBuf::from(home).join("Library/Caches/Bonk")),
        _ => var("XDG_CACHE_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|dir| dir.join("bonk")),
    }
}

/// Takes the OS as input, so each system's rule is tested everywhere.
fn config_file_for(os: &str, var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    match os {
        "windows" => var("APPDATA").map(|dir| PathBuf::from(dir).join("Bonk").join("config.json")),
        "macos" => var("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support/Bonk/config.json")),
        _ => var("XDG_CONFIG_HOME")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|dir| dir.join("bonk").join("config.json")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn un_peripherique_se_reconnait_a_sa_forme() {
        assert!(is_device_path("/dev/sdb"));
        assert!(is_device_path(r"\\.\PhysicalDrive1"));
        assert!(!is_device_path("image.img"));
        assert!(!is_device_path(r"C:\image.img"));
    }

    #[test]
    fn le_cache_suit_les_regles_de_chaque_systeme() {
        let path = |os, pairs| cache_dir_for(os, &env(pairs));
        assert_eq!(
            path("windows", &[("LOCALAPPDATA", "L")]),
            Some(PathBuf::from("L").join("Bonk").join("cache"))
        );
        assert_eq!(
            path("macos", &[("HOME", "H")]),
            Some(PathBuf::from("H").join("Library/Caches/Bonk"))
        );
        assert_eq!(
            path("linux", &[("XDG_CACHE_HOME", "X"), ("HOME", "H")]),
            Some(PathBuf::from("X").join("bonk"))
        );
        assert_eq!(
            path("linux", &[("XDG_CACHE_HOME", ""), ("HOME", "H")]),
            Some(PathBuf::from("H").join(".cache").join("bonk"))
        );
        assert_eq!(path("linux", &[]), None);
    }

    #[test]
    fn le_fichier_de_configuration_suit_les_regles_de_chaque_systeme() {
        let path = |os, pairs| config_file_for(os, &env(pairs));
        assert_eq!(
            path("windows", &[("APPDATA", "A")]),
            Some(PathBuf::from("A").join("Bonk").join("config.json"))
        );
        assert_eq!(
            path("macos", &[("HOME", "H")]),
            Some(PathBuf::from("H").join("Library/Application Support/Bonk/config.json"))
        );
        assert_eq!(
            path("linux", &[("XDG_CONFIG_HOME", "X"), ("HOME", "H")]),
            Some(PathBuf::from("X").join("bonk").join("config.json"))
        );
        // An empty variable counts as unset (XDG specification).
        assert_eq!(
            path("linux", &[("XDG_CONFIG_HOME", ""), ("HOME", "H")]),
            Some(
                PathBuf::from("H")
                    .join(".config")
                    .join("bonk")
                    .join("config.json")
            )
        );
        assert_eq!(path("linux", &[]), None);
    }
}
