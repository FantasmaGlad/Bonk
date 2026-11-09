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

/// Bounds are checked before each write: the kernel only answers "no space" if the disk vanished.
pub(super) fn support_lost(err: &BlockError) -> Option<JobError> {
    match err {
        BlockError::Io(io) if io.kind() == std::io::ErrorKind::StorageFull => Some(fail(
            Code::WriteFailed,
            "the drive disappeared during the write (unplugged, or a faulty USB port)",
        )),
        _ => None,
    }
}

pub(super) fn map_write_error(err: WriteError, format: Format) -> JobError {
    if let WriteError::Device(device) = &err
        && let Some(lost) = support_lost(device)
    {
        return lost;
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

/// A cancel requested by the system (Ctrl+C) wins.
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

    #[test]
    fn un_support_disparu_n_est_pas_un_disque_plein() {
        let full = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let error = map_write_error(WriteError::Device(BlockError::Io(full)), Format::Raw);
        assert_eq!(error.code, Code::WriteFailed);
        assert!(error.message.contains("unplugged"), "{}", error.message);
    }
}
