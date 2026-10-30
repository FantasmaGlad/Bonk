//! Ouverture de l'image à écrire.

use std::fs::{self, File};
use std::path::Path;

use bonk_core::codec::{self, CodecError, Source};

use super::types::{JobError, SourceInfo, fail};
use crate::errors::Code;

pub fn open_source(path: &Path) -> Result<(Source, SourceInfo), JobError> {
    let metadata = fs::metadata(path)
        .map_err(|e| fail(Code::SourceUnreadable, format!("{} : {e}", path.display())))?;
    if !metadata.is_file() {
        return Err(fail(
            Code::SourceUnreadable,
            format!("{} n'est pas un fichier ordinaire", path.display()),
        ));
    }
    let file = File::open(path)
        .map_err(|e| fail(Code::SourceUnreadable, format!("{} : {e}", path.display())))?;
    let source = codec::open(file, Some(metadata.len())).map_err(|e| match e {
        CodecError::Unsupported(_) => fail(Code::SourceFormatUnsupported, e),
        CodecError::Io(_) => fail(Code::SourceUnreadable, format!("{} : {e}", path.display())),
    })?;
    let info = SourceInfo {
        path: path.display().to_string(),
        format: source.format,
        size_bytes: source.output_len,
    };
    Ok((source, info))
}
