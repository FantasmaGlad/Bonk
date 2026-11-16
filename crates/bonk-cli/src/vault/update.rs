use std::io::Write;

use bonk_ops::job::{self, Deps, TraceIo, VaultUpdateDone, VaultUpdateJob};

use super::UpdateArgs;
use crate::catalog;
use crate::transfer::{finish, target_line};
use crate::vault::grub;

pub(super) fn run_update(
    args: &UpdateArgs,
    access: &catalog::Access,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let (boot, grub) = match grub(&args.grub, access) {
        Ok((boot, info)) => (boot, Some(info)),
        Err(error) => {
            return finish::<VaultUpdateDone>(Err(error), &args.common, out, err, |_, _| Ok(()));
        }
    };
    let job = VaultUpdateJob {
        target: args.common.target(),
        boot,
        grub,
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        // Without confirmation, the check alone.
        dry_run: args.dry_run || !args.yes,
        yes: args.yes,
        elevation: args.common.elevate.into(),
    };
    finish(
        job::vault_update(&job, deps),
        &args.common,
        out,
        err,
        |done, out| {
            let grub = done
                .grub
                .as_ref()
                .map_or_else(String::new, |grub| format!("GRUB {} ", grub.version));
            let state = if done.up_to_date {
                "already up to date"
            } else if done.updated {
                "updated"
            } else {
                "out of date: add --yes to update"
            };
            writeln!(out, "Vault on {}: {grub}{state}", target_line(&done.target))?;
            if done.verified {
                writeln!(out, "  Read-back: matches")?;
            }
            Ok(())
        },
    )
}
