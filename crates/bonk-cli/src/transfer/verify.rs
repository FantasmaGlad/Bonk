use std::io::Write;

use bonk_hal::format_size;

use super::args::*;
use super::output::*;
use super::write::cache_warning;
use crate::config::ViewDefaults;
use bonk_ops::job::{self, Deps, TraceIo, VerifyJob};
pub(crate) fn run_verify(
    args: &VerifyArgs,
    view: &ViewDefaults,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    let job = VerifyJob {
        target: args.common.target(),
        source: args.source.clone(),
        options: args.common.options(view),
        trace_io: args.common.trace_io.map(TraceIo::from),
        elevation: args.common.elevate.into(),
    };
    let result = {
        let mut progress = sink(
            progress_mode(args.common.progress, quiet),
            &mut *out,
            &mut *err,
        );
        job::verify(&job, deps, &mut progress)
    };
    finish(result, &args.common, out, err, verify_lines)
}

fn verify_lines(done: &job::VerifyDone, out: &mut dyn Write) -> std::io::Result<()> {
    writeln!(
        out,
        "Verification passed: {} identical to the image ({})",
        format_size(done.verify.bytes, bonk_hal::Lang::En),
        target_line(&done.target)
    )?;
    if done.ignored_ranges.is_empty() {
        cache_warning(&done.verify, out)?;
        return writeln!(out, "  SHA-256: {}", done.verify.sha256);
    }
    let octets: u64 = done.ignored_ranges.iter().map(|r| r.length).sum();
    writeln!(
        out,
        "  Excluded from the comparison: {octets} bytes of GPT fields its repair rewrites"
    )?;
    cache_warning(&done.verify, out)?;
    // Repaired bytes are zeroed on both sides, so this hash never matches the publisher's.
    writeln!(
        out,
        "  SHA-256 of the compared content, excluded bytes zeroed (it differs from the image's own hash): {}",
        done.verify.sha256
    )
}
