//! Ouverture de l'image à écrire.
//!
//! Le fichier est ouvert une seule fois ; empreinte, analyse, écriture et vérification le lisent par
//! le même descripteur, chacune à sa position. Un fichier remplacé sous le même nom entre deux étapes
//! ne peut donc pas faire contrôler d'autres octets que ceux qui sont écrits.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::Arc;

use bonk_core::codec::{self, CodecError, Source};

use super::types::{JobError, SourceInfo, fail};
use crate::errors::Code;

/// L'image ouverte : un seul descripteur, autant de lectures indépendantes que d'étapes.
pub struct ImageFile {
    file: Arc<File>,
    path: String,
    len: u64,
}

impl ImageFile {
    pub fn open(path: &Path) -> Result<Self, JobError> {
        let unreadable =
            |e: io::Error| fail(Code::SourceUnreadable, format!("{} : {e}", path.display()));
        let file = File::open(path).map_err(unreadable)?;
        // Contrôlé sur le descripteur ouvert, pas sur le chemin, qui a pu changer entre-temps.
        let metadata = file.metadata().map_err(unreadable)?;
        if !metadata.is_file() {
            return Err(fail(
                Code::SourceUnreadable,
                format!("{} n'est pas un fichier ordinaire", path.display()),
            ));
        }
        Ok(Self {
            file: Arc::new(file),
            path: path.display().to_string(),
            len: metadata.len(),
        })
    }

    pub fn size(&self) -> u64 {
        self.len
    }

    /// Les octets du fichier tels quels, depuis le début.
    pub fn raw(&self) -> FileReader {
        FileReader {
            file: Arc::clone(&self.file),
            position: 0,
        }
    }

    /// Les octets de l'image, décompressés, depuis le début.
    pub fn decoded(&self) -> Result<(Source, SourceInfo), JobError> {
        let source = codec::open(self.raw(), Some(self.len)).map_err(|e| match e {
            CodecError::Unsupported(_) => fail(Code::SourceFormatUnsupported, e),
            CodecError::Io(_) => fail(Code::SourceUnreadable, format!("{} : {e}", self.path)),
        })?;
        let info = SourceInfo {
            path: self.path.clone(),
            format: source.format,
            size_bytes: source.output_len,
        };
        Ok((source, info))
    }
}

/// Lecture séquentielle par accès positionnels : les lecteurs d'un même fichier ne partagent pas de
/// curseur.
pub struct FileReader {
    file: Arc<File>,
    position: u64,
}

impl Read for FileReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = read_at(&self.file, buf, self.position)?;
        self.position += n as u64;
        Ok(n)
    }
}

#[cfg(unix)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, buf, offset)
}

// `seek_read` déplace aussi le curseur du fichier, qu'aucun lecteur n'utilise.
#[cfg(windows)]
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, buf, offset)
}
