use std::io::Write;

use bonk_core::format::table::{PartitionPlan, TableKind};
use bonk_hal::format_size;
use clap::{Args, Subcommand};

use crate::config::ViewDefaults;
use crate::format::TableArg;
use crate::transfer::{CommonArgs, finish, target_line};
use bonk_ops::job::{
    self, Deps, TraceIo, VaultInitDone, VaultInitJob, VaultListDone, VaultListJob,
};

#[derive(Debug, Subcommand)]
pub(crate) enum VaultAction {
    /// Prepare a disk to carry several ISO images: exFAT data partition and EFI boot partition. Erases
    /// everything
    Init(InitArgs),
    /// ISO images on a vault (read-only)
    List(ListArgs),
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct InitArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Partition table: mbr boots in BIOS and UEFI up to 2 TiB, gpt is needed beyond
    #[arg(long, value_enum, default_value_t = TableKind::VAULT_DEFAULT.into())]
    table: TableArg,

    /// Confirm: without --yes or --force, Bonk does nothing
    #[arg(long)]
    yes: bool,

    /// Describe what would be done without writing anything
    #[arg(long)]
    dry_run: bool,

    /// Confirm like --yes, and allow an internal disk admitted by --show-internal, without a question.
    /// Never a system disk: the safety filter stays in charge
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct ListArgs {
    #[command(flatten)]
    common: CommonArgs,
}

impl VaultAction {
    pub(crate) fn touches_real_disk(&self) -> bool {
        match self {
            Self::Init(args) => !args.common.simulate && !args.dry_run,
            Self::List(args) => !args.common.simulate,
        }
    }
}

pub(crate) fn schema_init() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultInitDone)).unwrap_or_default()
}

pub(crate) fn schema_list() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultListDone)).unwrap_or_default()
}

fn size(part: PartitionPlan, block_size: u32) -> String {
    format_size(part.sectors() * u64::from(block_size), bonk_hal::Lang::En)
}

pub(crate) fn run(
    action: &VaultAction,
    view: &ViewDefaults,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    match action {
        VaultAction::Init(args) => run_init(args, deps, out, err),
        VaultAction::List(args) => run_list(args, view, deps, out, err),
    }
}

fn run_init(args: &InitArgs, deps: &Deps<'_>, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let job = VaultInitJob {
        target: args.common.target(),
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
        |done, out| {
            writeln!(
                out,
                "{} {}",
                if done.dry_run {
                    "Plan (nothing is written):"
                } else {
                    "Vault ready:"
                },
                target_line(&done.target),
            )?;
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
            if !done.unmounted.is_empty() {
                writeln!(out, "  Unmount: {}", done.unmounted.join(", "))?;
            }
            if done.verified {
                writeln!(out, "  Read-back: matches")?;
            }
            Ok(())
        },
    )
}

fn run_list(
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
