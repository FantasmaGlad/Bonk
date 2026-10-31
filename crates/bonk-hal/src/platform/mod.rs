//! Ce que Bonk demande au système d'exploitation, derrière une seule interface.
//!
//! Tout le code propre à un OS passe par [`Platform`] ; le choix de l'implémentation n'a lieu qu'une
//! fois, dans [`current`]. Ajouter un système revient à écrire une implémentation : les opérations et
//! les interfaces n'ont rien à changer.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod udisks;
mod unsupported;

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

/// Faut-il demander au système les droits qui manquent ?
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Elevation {
    /// Oui : l'autorisation du système apparaît au moment voulu.
    #[default]
    Auto,
    /// Non : droits de l'appelant seulement. Pour un service sans personne devant l'écran.
    Never,
}

/// Pourquoi une fonction manque.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Missing {
    /// Un outil de l'hôte manque : `detail` le nomme.
    MissingTool,
    /// Ce système ne le permet pas, ou Bonk ne le sait pas encore sur lui.
    OsUnsupported,
    /// Le système refuse à cet utilisateur.
    NoPermission,
}

/// Une fonction que l'hôte permet, ou pourquoi il ne la permet pas.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Capability {
    /// Nom stable : `write`, `format.fat32`...
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

/// Nom de la capacité de formatage d'un système de fichiers.
#[must_use]
pub fn format_capability(fs: Filesystem) -> String {
    format!("format.{}", fs.name())
}

/// Accès d'un système d'exploitation aux disques.
pub trait Platform: Send + Sync {
    /// Énumère les disques : des faits bruts, que [`crate::safety::evaluate`] juge ensuite.
    fn enumerate(&self) -> Result<Vec<Disk>, EnumerateError>;

    /// Comme [`Platform::enumerate`], et rend chaque lecture de source système avec son contenu.
    fn enumerate_recorded(&self) -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>);

    /// Énumère à partir d'une capture de sources, sans toucher au système.
    fn enumerate_from_capture(&self, reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError>;

    /// Démonte tout ce que porte le disque et rend les points démontés ; s'arrête au premier échec.
    /// Avec [`Elevation::Never`], aucune autorisation n'est demandée au système.
    fn unmount(&self, disk: &Disk, elevation: Elevation) -> Result<Vec<String>, OpenTargetError>;

    /// Obtient le droit d'ouvrir le disque, sans le garder ouvert. **Appelé avant tout démontage** : un
    /// refus ne doit laisser aucune trace sur la clé.
    fn prepare_access(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), BlockError>;

    /// Ouvre un disque entier en accès brut. N'est appelé qu'après les garde-fous de `bonk-ops`.
    /// `cancelled` abandonne l'attente d'une autorisation du système (`ErrorKind::Interrupted`).
    fn open_disk(
        &self,
        path: &Path,
        access: Access,
        elevation: Elevation,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Box<dyn BlockDevice>, BlockError>;

    /// Ce que ce système permet, et pourquoi pas le reste (`bonk capabilities`).
    fn capabilities(&self) -> Vec<Capability>;

    /// Formate une partition avec les outils de l'hôte (exFAT, NTFS, ext4). Seul le résultat de
    /// l'outil compte, sa sortie n'est pas interprétée.
    fn format_partition(
        &self,
        partition: &Path,
        fs: Filesystem,
        label: Option<&str>,
        elevation: Elevation,
    ) -> Result<(), BlockError>;

    /// Demande au système de relire la table de partitions du disque, qu'on vient de remplacer : sans
    /// cela, il peut montrer un moment les anciennes partitions, ou aucune.
    fn rescan(&self, disk: &Path, elevation: Elevation) -> Result<(), BlockError>;

    /// Le processus tourne-t-il en administrateur ? La fenêtre refuse alors de s'ouvrir.
    fn is_privileged(&self) -> bool;

    /// Version du noyau, pour le rapport de diagnostic, quand le système la donne.
    fn kernel_release(&self) -> Option<String>;
}

/// L'implémentation du système en cours.
#[must_use]
pub fn current() -> &'static dyn Platform {
    #[cfg(target_os = "linux")]
    {
        &linux::Linux
    }
    #[cfg(not(target_os = "linux"))]
    {
        &Unsupported
    }
}

/// Emplacement habituel du fichier de configuration sur le système en cours.
#[must_use]
pub fn config_file(var: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    config_file_for(std::env::consts::OS, var)
}

/// Sépare le choix du système de sa lecture : les règles de chaque système se testent partout.
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
        // Une variable vide équivaut à son absence (spécification XDG).
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
