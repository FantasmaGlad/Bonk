use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use bonk_catalog::manifest;
use bonk_core::format::table::{PartitionPlan, TableKind};
use bonk_core::vault::{self as core_vault, BootFile};
use bonk_hal::format_size;
use clap::{Args, Subcommand};

use crate::catalog;
use crate::config::ViewDefaults;
use crate::format::TableArg;
use crate::transfer::{CommonArgs, finish, target_line};
use bonk_ops::job::{
    self, Deps, GrubInfo, JobError, TraceIo, VaultInitDone, VaultInitJob, VaultListDone,
    VaultListJob,
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

    /// Folder holding BOOTX64.EFI, for a machine without network: checked against the hash of the
    /// last catalog seen
    #[arg(long, value_name = "DIR")]
    grub_dir: Option<PathBuf>,

    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL")]
    catalog_url: Option<String>,

    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
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
    access: &catalog::Access,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    bonk_hal::interrupt::install();
    match action {
        VaultAction::Init(args) => run_init(args, access, deps, out, err),
        VaultAction::List(args) => run_list(args, view, deps, out, err),
    }
}

/// Architecture of the GRUB written: the one the vault boots today.
const GRUB_ARCH: &str = "x86_64";

/// GRUB named by the signed catalog: from its address through the cache, or from `--grub-dir`, always
/// checked against the signed hash.
fn grub(args: &InitArgs, access: &catalog::Access) -> Result<(Vec<BootFile>, GrubInfo), JobError> {
    let (loaded, dir, base) = catalog::load_from(
        access,
        args.catalog_url.as_deref(),
        args.cache_dir.as_deref(),
    )?;
    let entry = loaded
        .catalog
        .grub
        .ok_or_else(|| catalog::refused("the catalog does not offer GRUB yet: try again later"))?;
    let asset = entry
        .efi
        .get(GRUB_ARCH)
        .ok_or_else(|| catalog::refused(format!("the catalog offers no GRUB for {GRUB_ARCH}")))?;
    let data = match &args.grub_dir {
        Some(folder) => local_grub(folder, asset, &entry.version)?,
        None => {
            let path = manifest::asset_path(&base, &dir, "grub", asset, &*access.fetch)
                .map_err(|e| catalog::refused(format!("GRUB {}: {e}", entry.version)))?;
            fs::read(&path).map_err(|e| catalog::refused(format!("{}: {e}", path.display())))?
        }
    };
    let files = core_vault::boot_files(&[(GRUB_ARCH, data)])
        .map_err(|e| catalog::refused(e.to_string()))?;
    let info = GrubInfo {
        version: entry.version,
        build: entry.build,
        arch: GRUB_ARCH.to_owned(),
        sha256: asset.sha256.clone(),
    };
    Ok((files, info))
}

fn local_grub(folder: &Path, asset: &manifest::Asset, version: &str) -> Result<Vec<u8>, JobError> {
    let name = core_vault::efi_file_name(GRUB_ARCH).unwrap_or("BOOTX64.EFI");
    let path = folder.join(name);
    let data = fs::read(&path).map_err(|e| catalog::refused(format!("{}: {e}", path.display())))?;
    if !manifest::asset_matches(asset, &data) {
        return Err(catalog::refused(format!(
            "{} is not the GRUB {version} of the catalog (hash differs)",
            path.display()
        )));
    }
    Ok(data)
}

fn run_init(
    args: &InitArgs,
    access: &catalog::Access,
    deps: &Deps<'_>,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    // A plan, or a refusal for want of confirmation, writes nothing: neither needs the network nor GRUB.
    let (boot, grub) = if args.dry_run || !(args.yes || args.force) {
        (Vec::new(), None)
    } else {
        match grub(args, access) {
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
            if let Some(grub) = &done.grub {
                writeln!(
                    out,
                    "  GRUB {} ({}), build {}, sha256 {}",
                    grub.version, grub.arch, grub.build, grub.sha256
                )?;
            }
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
