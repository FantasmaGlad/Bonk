use std::io::Write;

use bonk_hal::format_size;
use std::path::PathBuf;

use bonk_core::write::{Control, Progress};
use bonk_ops::errors::Code;
use bonk_ops::job::{self, Deps, JobError, TraceIo, VaultAddJob};

use super::AddArgs;
use crate::catalog::{Destination, download};
use crate::transfer::{finish, progress_mode, sink, target_line};
use crate::vault::{arch_name, family_name};

pub(super) fn run_add(
    args: &AddArgs,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let result = {
        let mut progress = sink(
            progress_mode(args.common.progress, quiet),
            &mut *out,
            &mut *err,
        );
        source(args, deps, &mut progress).and_then(|source| {
            let job = VaultAddJob {
                target: args.common.target(),
                source,
                name: None,
                options: args.common.write_options(),
                trace_io: args.common.trace_io.map(TraceIo::from),
                yes: args.yes,
                elevation: args.common.elevate.into(),
            };
            job::vault_add(&job, deps, &mut progress)
        })
    };
    finish(result, &args.common, out, err, |done, out| {
        writeln!(
            out,
            "Added to the vault on {}: {}",
            target_line(&done.target),
            done.name
        )?;
        writeln!(
            out,
            "  {}, {}, {}",
            format_size(done.size_bytes, bonk_hal::Lang::En),
            family_name(done.family),
            arch_name(done.arch)
        )?;
        writeln!(out, "  sha256 {}", done.sha256)?;
        if done.verified {
            writeln!(out, "  Read-back: matches")?;
        }
        Ok(())
    })
}

/// Nothing is downloaded without --yes.
fn source(
    args: &AddArgs,
    deps: &Deps<'_>,
    progress: &mut dyn FnMut(&Progress) -> Control,
) -> Result<PathBuf, JobError> {
    let Some(id) = &args.image else {
        return Ok(args.iso.clone().unwrap_or_default());
    };
    if !args.yes {
        return Err(JobError {
            code: Code::ConfirmationRequired,
            message: "nothing downloaded or written without --yes".to_owned(),
        });
    }
    let request = download::Request {
        id,
        catalog_url: args.catalog_url.as_deref(),
        cache_dir: args.cache_dir.as_deref(),
        destination: Destination::Vault,
    };
    download::fetch_checked(&request, deps, progress).map(|kept| PathBuf::from(kept.path))
}
