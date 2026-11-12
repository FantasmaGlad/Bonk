use std::io::Write;

use super::args::*;
use super::output::*;
use bonk_ops::job::{self, Deps, RepairJob, TraceIo};
pub(crate) fn run_repair(
    args: &RepairArgs,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = RepairJob {
        target: args.common.target(),
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        elevation: args.common.elevate.into(),
        force: args.force,
    };
    let result = job::repair(&job, deps);
    finish(result, &args.common, out, err, repair_lines)
}

fn repair_lines(done: &job::RepairDone, out: &mut dyn Write) -> std::io::Result<()> {
    let state = if done.needs_repair {
        "GPT needs repair"
    } else {
        "GPT in order"
    };
    writeln!(out, "{}: {state}", target_line(&done.target))?;
    if !done.findings_before.is_empty() {
        writeln!(
            out,
            "  Findings before: {}",
            done.findings_before.join(", ")
        )?;
    }
    if let Some(r) = &done.repair {
        writeln!(out, "  Repaired: {}", r.actions.join(", "))?;
    }
    if let Some(after) = &done.findings_after {
        let after = if after.is_empty() {
            "none".to_owned()
        } else {
            after.join(", ")
        };
        writeln!(out, "  Findings after: {after}")?;
    }
    Ok(())
}
