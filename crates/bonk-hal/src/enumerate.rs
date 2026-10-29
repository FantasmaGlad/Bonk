//! Énumération des disques de l'hôte.
//!
//! [`enumerate_disks`] renvoie des **faits bruts** ([`Disk`]) ; la décision de proposer ou d'écarter
//! un disque appartient à [`crate::safety::evaluate`].
//!
//! # Échouer plutôt que deviner
//! Si une source dont dépend la sécurité est illisible (points de montage, base udev, liste des
//! disques), l'énumération échoue : proposer un disque sans savoir s'il est monté serait pire que ne
//! rien proposer. Les attributs purement descriptifs (modèle, étiquette) ont, eux, une valeur par
//! défaut.

use std::error::Error;
use std::fmt;
use std::io;

use schemars::JsonSchema;
use serde::Serialize;

use crate::disk::Disk;

/// Échec de l'énumération.
#[derive(Debug)]
pub enum EnumerateError {
    /// Une source système nécessaire est illisible.
    Io { what: String, source: io::Error },
    /// Une source système est lisible mais son contenu n'a pas la forme attendue.
    Malformed { what: String, detail: String },
    /// Pas encore d'énumération pour ce système d'exploitation.
    Unsupported { os: &'static str },
}

impl fmt::Display for EnumerateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { what, source } => write!(f, "lecture impossible de {what} : {source}"),
            Self::Malformed { what, detail } => write!(f, "{what} inattendu : {detail}"),
            Self::Unsupported { os } => write!(
                f,
                "l'énumération des disques n'est pas encore disponible sous {os}"
            ),
        }
    }
}

impl Error for EnumerateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Énumère les disques du système en cours.
///
/// # Errors
/// [`EnumerateError`] si une source nécessaire à la sécurité est illisible, ou si le système n'est
/// pas encore pris en charge.
pub fn enumerate_disks() -> Result<Vec<Disk>, EnumerateError> {
    #[cfg(target_os = "linux")]
    {
        crate::linux::enumerate(&crate::linux::LinuxRoots::system())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(EnumerateError::Unsupported {
            os: std::env::consts::OS,
        })
    }
}

/// Une lecture d'une source système, telle qu'elle s'est passée.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct SourceRead {
    pub path: String,
    /// `file` (contenu lu) ou `dir` (liste des entrées).
    pub kind: &'static str,
    pub ok: bool,
    pub bytes: Option<usize>,
    pub error: Option<String>,
    pub micros: u64,
    /// Contenu (fichier) ou entrées (répertoire), tronqué ; seulement quand on enregistre.
    pub content: Option<String>,
}

/// Comme [`enumerate_disks`], et rend aussi chaque lecture de source système avec son contenu, pour
/// `bonk debug sources`. Hors Linux, aucune lecture n'est enregistrée (pas encore d'énumération).
pub fn enumerate_disks_recorded() -> (Result<Vec<Disk>, EnumerateError>, Vec<SourceRead>) {
    #[cfg(target_os = "linux")]
    {
        crate::linux::enumerate_recorded(&crate::linux::LinuxRoots::system())
    }
    #[cfg(not(target_os = "linux"))]
    {
        (
            Err(EnumerateError::Unsupported {
                os: std::env::consts::OS,
            }),
            Vec::new(),
        )
    }
}
