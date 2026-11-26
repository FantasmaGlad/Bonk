use std::io::{self, Write};

use bonk_core::vault::VaultBoot;
use bonk_ops::job::{self, Deps, TraceIo, VaultInitDone, VaultInitJob};

use super::{InitArgs, size};
use crate::catalog;
use crate::transfer::{finish, target_line};
use crate::vault::grub;

pub(super) fn run_init(
    args: &InitArgs,
    access: &catalog::Access,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    // A plan, or a refusal for want of confirmation, writes nothing: neither needs the network nor GRUB.
    let (boot, grub) = if args.dry_run || !(args.yes || args.force) {
        (VaultBoot::default(), None)
    } else {
        match grub(&args.grub, access) {
            Ok((boot, info)) => (boot, Some(info)),
            Err(error) => {
                return finish::<VaultInitDone>(Err(error), &args.common, out, err, |_, _| Ok(()));
            }
        }
    };
    let job = VaultInitJob {
        target: args.common.target(),
        boot,
        grub,
        table: args.table.into(),
        options: args.common.write_options(),
        trace_io: args.common.trace_io.map(TraceIo::from),
        dry_run: args.dry_run,
        yes: args.yes,
        force: args.force,
        elevation: args.common.elevate.into(),
    };
    finish(
        job::vault_init(&job, deps),
        &args.common,
        out,
        err,
        init_lines,
    )
}

pub(super) fn init_lines(done: &VaultInitDone, out: &mut dyn Write) -> io::Result<()> {
    let head = if done.dry_run {
        "Plan (nothing is written):"
    } else {
        "Vault ready:"
    };
    writeln!(out, "{head} {}", target_line(&done.target))?;
    writeln!(out, "  Table: {}", done.layout.table.name())?;
    writeln!(
        out,
        "  Partition 1: {}, exFAT, {}: copy the ISO images here",
        done.vault_label,
        size(done.layout.vault, done.block_size)
    )?;
    writeln!(
        out,
        "  Partition 2: {}, FAT32, EFI system partition, {}",
        done.efi_label,
        size(done.layout.efi, done.block_size)
    )?;
    if let Some(grub) = &done.grub {
        writeln!(out, "  GRUB {}, build {}", grub.version, grub.build)?;
        for (arch, sha256) in &grub.efi {
            writeln!(out, "    {arch}: sha256 {sha256}")?;
        }
        let bios = if done.bios {
            "BIOS: GRUB in the MBR"
        } else {
            "BIOS: no, this vault starts by UEFI only"
        };
        writeln!(out, "    {bios}")?;
    }
    if !done.unmounted.is_empty() {
        writeln!(out, "  Unmount: {}", done.unmounted.join(", "))?;
    }
    if done.verified {
        writeln!(out, "  Read-back: matches")?;
    }
    Ok(())
}
