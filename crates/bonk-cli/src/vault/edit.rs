use std::io::Write;

use bonk_hal::format_size;
use bonk_ops::job::{self, Deps, TraceIo, VaultAddJob, VaultRemoveJob};

use super::{AddArgs, RemoveArgs};
use crate::transfer::{finish, progress_mode, sink, target_line};
use crate::vault::{arch_name, family_name};

pub(super) fn run_add(
    args: &AddArgs,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let job = VaultAddJob {
        target: args.common.target(),
        source: args.iso.clone(),
        name: None,
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        yes: args.yes,
        elevation: args.common.elevate.into(),
    };
    let result = {
        let mut progress = sink(
            progress_mode(args.common.progress, quiet),
            &mut *out,
            &mut *err,
        );
        job::vault_add(&job, deps, &mut progress)
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

pub(super) fn run_remove(
    args: &RemoveArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let job = VaultRemoveJob {
        target: args.common.target(),
        name: args.name.clone(),
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        yes: args.yes,
        elevation: args.common.elevate.into(),
    };
    finish(
        job::vault_remove(&job, deps),
        &args.common,
        out,
        err,
        |done, out| {
            writeln!(
                out,
                "Removed from the vault on {}: {} ({} freed)",
                target_line(&done.target),
                done.name,
                format_size(done.freed_bytes, bonk_hal::Lang::En)
            )
        },
    )
}
