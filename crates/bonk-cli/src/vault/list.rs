use std::io::Write;

use bonk_core::vault as core_vault;
use bonk_hal::format_size;
use bonk_ops::job::{self, Deps, TraceIo, VaultListJob};

use super::ListArgs;
use crate::config::ViewDefaults;
use crate::transfer::{finish, target_line};

pub(super) fn run_list(
    args: &ListArgs,
    view: &ViewDefaults,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let job = VaultListJob {
        target: args.common.target(),
        options: args.common.options(view),
        trace_io: args.common.trace_io.map(TraceIo::from),
        elevation: args.common.elevate.into(),
    };
    finish(
        job::vault_list(&job, deps),
        &args.common,
        out,
        err,
        |done, out| {
            writeln!(
                out,
                "Vault on {} ({}): {} ISO image{}",
                target_line(&done.target),
                done.layout.table.name(),
                done.images.len(),
                if done.images.len() == 1 { "" } else { "s" }
            )?;
            for image in &done.images {
                writeln!(
                    out,
                    "  {}  {}",
                    format_size(image.size_bytes, bonk_hal::Lang::En),
                    image.path
                )?;
            }
            Ok(())
        },
    )
}

pub(crate) fn arch_name(arch: core_vault::IsoArch) -> &'static str {
    match arch {
        core_vault::IsoArch::X86_64 => "for x86_64 PCs",
        core_vault::IsoArch::Aarch64 => "for ARM64 machines",
    }
}
