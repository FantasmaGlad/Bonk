//! Traduction des erreurs du moteur en codes stables, et progression sous annulation.

use bonk_core::BlockError;
use bonk_core::codec::Format;
use bonk_core::write::{Control, Progress, VerifyError, WriteError};

use super::types::{Deps, JobError, fail};
use crate::errors::Code;

fn source_error(format: Format) -> Code {
    if format == Format::Raw {
        Code::SourceUnreadable
    } else {
        Code::DecompressionFailed
    }
}

pub(super) fn map_write_error(err: WriteError, format: Format) -> JobError {
    // Les bornes sont contrôlées avant chaque écriture : le noyau ne répond « plus de place » que si le disque
    // a disparu sous l'opération.
    if let WriteError::Device(BlockError::Io(io)) = &err
        && io.kind() == std::io::ErrorKind::StorageFull
    {
        return fail(
            Code::WriteFailed,
            "le support a disparu pendant l'écriture (débranché, ou port USB défaillant)",
        );
    }
    let code = match &err {
        WriteError::Cancelled { .. } => Code::Cancelled,
        WriteError::TooLarge { .. } => Code::SourceTooLarge,
        WriteError::Source(_) => source_error(format),
        WriteError::Device(BlockError::Io(io))
            if io.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            Code::DeviceOpenDenied
        }
        WriteError::Device(_) => Code::WriteFailed,
    };
    fail(code, err)
}

pub(super) fn map_verify_error(err: VerifyError, format: Format) -> JobError {
    let code = match &err {
        VerifyError::Mismatch { .. } => Code::VerifyMismatch,
        VerifyError::Cancelled => Code::Cancelled,
        VerifyError::Source(_) => source_error(format),
        VerifyError::Device(_) => Code::WriteFailed,
    };
    fail(code, err)
}

/// Enveloppe le rappel de progression : l'annulation demandée par le système (Ctrl+C) l'emporte.
pub(super) fn guarded<'a>(
    deps: &'a Deps<'a>,
    progress: &'a mut dyn FnMut(&Progress) -> Control,
) -> impl FnMut(&Progress) -> Control + 'a {
    move |p| {
        if (deps.io.cancelled)() {
            Control::Cancel
        } else {
            progress(p)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le noyau répond « plus de place » sur un disque débranché : dire ce qui s'est passé.
    #[test]
    fn un_support_disparu_n_est_pas_un_disque_plein() {
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let error = map_write_error(WriteError::Device(BlockError::Io(full)), Format::Raw);
        assert_eq!(error.code, Code::WriteFailed);
        assert!(error.message.contains("débranché"), "{}", error.message);
    }
}
