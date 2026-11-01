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
use serde::{Deserialize, Serialize};

use crate::disk::Disk;

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
            Self::Io { what, source } => write!(f, "could not read {what}: {source}"),
            Self::Malformed { what, detail } => write!(f, "unexpected {what}: {detail}"),
            Self::Unsupported { os } => write!(f, "disk enumeration is not available yet on {os}"),
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
pub fn enumerate_disks() -> Result<Vec<Disk>, EnumerateError> {
    crate::platform::current().enumerate()
}

/// Une lecture d'une source système, telle qu'elle s'est passée.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SourceRead {
    pub path: String,
    /// `file` (contenu lu), `dir` (liste des entrées) ou `link` (lien résolu : le contenu est la cible
    /// canonique relative à `/sys`).
    pub kind: String,
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
    crate::platform::current().enumerate_recorded()
}

/// Énumère les disques à partir d'une capture de sources (`bonk diagnose --capture`), sans toucher au
/// système : sert à rejouer ailleurs un cas de support (`--fixture`) et à écrire des tests sur de vraies
/// machines. Une capture de Windows ou de macOS se rejoue partout ; celle d'un Linux, sur Linux.
pub fn enumerate_disks_from_capture(reads: &[SourceRead]) -> Result<Vec<Disk>, EnumerateError> {
    if let Some(result) = crate::windows_facts::from_capture(reads) {
        return result;
    }
    if let Some(result) = crate::macos_facts::from_capture(reads) {
        return result;
    }
    crate::platform::current().enumerate_from_capture(reads)
}
