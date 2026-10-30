//! Traduction des erreurs du moteur en codes stables, et progression sous annulation.

use bonk_core::BlockError;
use bonk_core::codec::Format;
use bonk_core::write::{Control, Progress, VerifyError, WriteError};

use super::types::{Deps, JobError, fail};
use crate::errors::Code;

fn source_error(format: Format, err: &std::io::Error) -> Code {
    if format == Format::Raw {
        let _ = err;
        Code::SourceUnreadable
    } else {
        Code::DecompressionFailed
    }
}

pub(super) fn map_write_error(err: WriteError, format: Format) -> JobError {
    let code = match &err {
        WriteError::Cancelled { .. } => Code::Cancelled,
        WriteError::TooLarge { .. } => Code::SourceTooLarge,
        WriteError::Source(io) => source_error(format, io),
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
        VerifyError::Source(io) => source_error(format, io),
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
