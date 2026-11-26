use std::io::Write;
use std::path::PathBuf;

use bonk_core::format::table::{PartitionPlan, TableKind};
use bonk_hal::format_size;
use clap::{Args, Subcommand};

use crate::catalog;
use crate::config::ViewDefaults;
use crate::format::TableArg;
use crate::transfer::CommonArgs;
use bonk_ops::job::{
    Deps, VaultAddDone, VaultInitDone, VaultListDone, VaultRemoveDone, VaultUpdateDone,
};

mod add;
mod grub;
mod init;
mod list;
mod remove;
mod update;

use add::run_add;
pub(crate) use grub::grub;
use init::*;
use list::*;
use remove::run_remove;
use update::*;

#[derive(Debug, Subcommand)]
pub(crate) enum VaultAction {
    /// Prepare a disk to carry several ISO images: exFAT data partition and EFI boot partition. Erases
    /// everything
    Init(InitArgs),
    /// ISO images on a vault (read-only)
    List(ListArgs),
    /// Copy an ISO to the vault, in one piece, after checking the vault menu can start it; reads it back
    Add(AddArgs),
    /// Remove an ISO from the vault, by its name
    Remove(RemoveArgs),
    /// Replace GRUB and the vault menu with those of the signed catalog when they differ; ISO images stay
    Update(UpdateArgs),
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

    #[command(flatten)]
    grub: GrubSource,
}

/// Where GRUB comes from, for `init` and `update`.
#[derive(Debug, Args, Default)]
pub(crate) struct GrubSource {
    /// Folder holding the GRUB images (BOOTX64.EFI, BOOTAA64.EFI, bios/boot.img, bios/core.img), for a machine
    /// without network: each checked against the hash of the last catalog seen; without bios/core.img, the
    /// vault starts by UEFI only
    #[arg(long, value_name = "DIR")]
    pub(crate) grub_dir: Option<PathBuf>,

    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL")]
    pub(crate) catalog_url: Option<String>,

    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    pub(crate) cache_dir: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct UpdateArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Confirm: without --yes, Bonk only says whether an update is needed
    #[arg(long)]
    yes: bool,

    /// Say whether an update is needed without writing anything
    #[arg(long)]
    dry_run: bool,

    #[command(flatten)]
    grub: GrubSource,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct ListArgs {
    #[command(flatten)]
    common: CommonArgs,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct AddArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// ISO image to copy: Debian live, Ubuntu, Arch Linux, Fedora live or an Anaconda installer (AlmaLinux,
    /// Rocky Linux); spaces in its name become _
    #[arg(required_unless_present = "image", conflicts_with = "image")]
    iso: Option<PathBuf>,

    /// Image of the signed catalog (`bonk catalog list`): downloaded into the cache, checked, then added
    #[arg(long, value_name = "ID")]
    image: Option<String>,

    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL", requires = "image")]
    catalog_url: Option<String>,

    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR", requires = "image")]
    cache_dir: Option<PathBuf>,

    /// Confirm: without --yes, Bonk does nothing
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct RemoveArgs {
    #[command(flatten)]
    common: CommonArgs,

    /// Name of the ISO at the root of the vault, as `bonk vault list` shows it
    name: String,

    /// Confirm: without --yes, Bonk does nothing
    #[arg(long)]
    yes: bool,
}

impl VaultAction {
    pub(crate) fn touches_real_disk(&self) -> bool {
        match self {
            Self::Init(args) => !args.common.simulate && !args.dry_run,
            Self::List(args) => !args.common.simulate,
            Self::Add(args) => !args.common.simulate,
            Self::Remove(args) => !args.common.simulate,
            Self::Update(args) => !args.common.simulate,
        }
    }
}

pub(crate) fn schema_init() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultInitDone)).unwrap_or_default()
}

pub(crate) fn schema_list() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultListDone)).unwrap_or_default()
}

pub(crate) fn schema_add() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultAddDone)).unwrap_or_default()
}

pub(crate) fn schema_update() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultUpdateDone)).unwrap_or_default()
}

pub(crate) fn schema_remove() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(VaultRemoveDone)).unwrap_or_default()
}

fn size(part: PartitionPlan, block_size: u32) -> String {
    format_size(part.sectors() * u64::from(block_size), bonk_hal::Lang::En)
}

pub(crate) fn run(
    action: &VaultAction,
    view: &ViewDefaults,
    access: &catalog::Access,
    deps: &Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    match action {
        VaultAction::Init(args) => run_init(args, access, deps, out, err),
        VaultAction::List(args) => run_list(args, view, deps, out, err),
        VaultAction::Add(args) => run_add(args, deps, quiet, out, err),
        VaultAction::Remove(args) => run_remove(args, deps, out, err),
        VaultAction::Update(args) => run_update(args, access, deps, out, err),
    }
}
