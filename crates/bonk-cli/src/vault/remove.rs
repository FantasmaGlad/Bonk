use std::io::Write;

use bonk_hal::format_size;
use bonk_ops::job::{self, Deps, TraceIo, VaultRemoveJob};

use super::RemoveArgs;
use crate::transfer::{finish, target_line};

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
