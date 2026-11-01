//! Arguments communs de `bonk write`, `bonk verify` et `bonk repair-gpt`.

use std::path::PathBuf;

use bonk_hal::{Elevation, FilterOptions};
use clap::{Args, ValueEnum};

use crate::config::ViewDefaults;
use crate::format::TableArg;
use bonk_ops::job::{Target, TraceIo, WriteMode};

/// Niveau de `--trace-io` (voir [`TraceIo`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum TraceIoArg {
    /// Offset, length, duration and checksum of each access
    Metadata,
    /// With the first 16 bytes: for image files only
    Data,
}

impl From<TraceIoArg> for TraceIo {
    fn from(arg: TraceIoArg) -> Self {
        match arg {
            TraceIoArg::Metadata => Self::Metadata,
            TraceIoArg::Data => Self::Data,
        }
    }
}

/// Valeur de `--elevate` (voir [`Elevation`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ElevateArg {
    /// Ask for system authorization when opening the disk
    Auto,
    /// The caller's rights only
    Never,
}

impl From<ElevateArg> for Elevation {
    fn from(arg: ElevateArg) -> Self {
        match arg {
            ElevateArg::Auto => Self::Auto,
            ElevateArg::Never => Self::Never,
        }
    }
}

/// Mode de `--mode` (voir [`WriteMode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ModeArg {
    /// Extract a Windows ISO; refuse a truncated image or one that would not boot as written
    Auto,
    /// Write the bytes as they are, whatever the analysis says
    Raw,
    /// Copy the files of a Windows installation ISO to a FAT32 partition; auto chooses it by itself
    Extract,
}

impl From<ModeArg> for WriteMode {
    fn from(arg: ModeArg) -> Self {
        match arg {
            ModeArg::Auto => Self::Auto,
            ModeArg::Raw => Self::Raw,
            ModeArg::Extract => Self::Extract,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ProgressMode {
    /// A progress line on standard error if it is a terminal, nothing otherwise
    Auto,
    /// One JSON event per line on standard output
    Json,
    /// No progress
    None,
}

/// Options communes aux opérations sur un disque.
#[derive(Debug, Args)]
pub(crate) struct CommonArgs {
    /// Stable identifier of the disk (see `bonk list`); with --simulate, path of the target image file
    pub(crate) target: String,

    /// The target is an image file (created if missing, with --size), not a disk: replays
    /// the operation without hardware
    #[arg(long)]
    pub(crate) simulate: bool,

    /// Size of the target image created by --simulate (decimal or 0x...); by default that of the source
    /// image when it is raw
    #[arg(long, value_name = "BYTES", value_parser = crate::debug::parse_number)]
    pub(super) size: Option<u64>,

    /// Allow internal non-system disks (stronger warning)
    #[arg(long)]
    pub(super) show_internal: bool,

    /// Test mode: allow virtual disks
    #[arg(long)]
    pub(super) allow_virtual: bool,

    /// Log each block access (offset, duration, checksum); `=data` adds the
    /// first bytes, for an image file only
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "metadata", require_equals = true)]
    pub(crate) trace_io: Option<TraceIoArg>,

    /// Missing rights: auto (ask the system) or never (the caller's rights only)
    #[arg(long, value_enum, default_value_t = ElevateArg::Auto)]
    pub(crate) elevate: ElevateArg,

    /// Progress: auto, json (one event per line) or none
    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    pub(super) progress: ProgressMode,

    /// Result as JSON
    #[arg(long)]
    pub(super) json: bool,
}

impl CommonArgs {
    pub(crate) fn target(&self) -> Target {
        if self.simulate {
            Target::Image {
                path: PathBuf::from(&self.target),
                size: self.size,
            }
        } else {
            Target::Disk(self.target.clone())
        }
    }

    pub(super) fn options(&self, view: &ViewDefaults) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal || view.show_internal,
            allow_virtual: self.allow_virtual || view.allow_virtual,
        }
    }

    /// Pour écrire, seules les options de la commande comptent : un réglage ou une variable oubliés règlent
    /// l'affichage, ils n'autorisent jamais à effacer un disque de plus.
    pub(crate) fn write_options(&self) -> FilterOptions {
        FilterOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
        }
    }
}

/// Write an image to a disk (or an image file with --simulate), then verify it
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct WriteArgs {
    #[command(flatten)]
    pub(super) common: CommonArgs,

    /// Image to write: a file (.img, .iso, .gz, .xz, .zst; the format is recognized by its content) or
    /// an https:// address to download, with its hash
    #[arg(required_unless_present = "image", conflicts_with = "image")]
    pub(super) source: Option<PathBuf>,

    /// Image of the signed catalog (`bonk catalog list`): its official link and hash are used
    #[arg(long, value_name = "ID", conflicts_with_all = ["sha256", "sha256_file"])]
    pub(super) image: Option<String>,

    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL", requires = "image")]
    pub(super) catalog_url: Option<String>,

    /// Confirm the write: without --yes or --force, Bonk does nothing (it never asks a question)
    #[arg(long)]
    pub(super) yes: bool,

    /// Describe what would be done without writing to the drive; an address is still downloaded
    /// into the cache, since the analysis needs the image
    #[arg(long)]
    pub(super) dry_run: bool,

    /// Published SHA-256 hash of this file: checked before touching the drive
    #[arg(long, value_name = "HEX", conflicts_with = "sha256_file")]
    pub(super) sha256: Option<String>,

    /// Published hash file (SHA256SUMS), path or https:// address: the hash is read under the name of
    /// the image
    #[arg(long, value_name = "PATH_OR_URL")]
    pub(super) sha256_file: Option<String>,

    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    pub(super) cache_dir: Option<PathBuf>,

    /// auto: analyze the image, extract a Windows ISO and refuse what would not boot; raw: write the
    /// bytes as they are; extract: require a Windows ISO
    #[arg(long, value_enum, default_value_t = ModeArg::Auto)]
    pub(super) mode: ModeArg,

    /// Partition table of a Windows drive; boot stays UEFI in both cases
    #[arg(long, value_enum, default_value_t = TableArg::Gpt)]
    pub(super) table: TableArg,

    /// Do not read the drive back after writing
    #[arg(long)]
    pub(super) no_verify: bool,

    /// Do not repair the backup GPT after writing
    #[arg(long)]
    pub(super) no_repair_gpt: bool,

    /// Confirm like --yes, and allow an internal disk admitted by --show-internal, without a question.
    /// Never a system disk: the safety filter stays in charge
    #[arg(long)]
    pub(super) force: bool,
}

impl WriteArgs {
    /// Cette commande ouvrirait-elle un vrai disque ? (`--dry-run` et `--simulate` : non)
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate && !self.dry_run
    }
}

/// Compare a disk (or an image) to an image, writing nothing
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct VerifyArgs {
    #[command(flatten)]
    pub(super) common: CommonArgs,

    /// Reference image
    pub(super) source: PathBuf,
}

impl VerifyArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate
    }
}

/// Repair the GPT of a disk (or an image): backup at the end of the drive, primary restored
#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct RepairArgs {
    #[command(flatten)]
    pub(super) common: CommonArgs,

    /// Confirm the repair
    #[arg(long)]
    pub(super) yes: bool,

    /// Say whether there is anything to repair, writing nothing
    #[arg(long)]
    pub(super) dry_run: bool,

    /// Confirm like --yes, and allow an internal disk admitted by --show-internal, without a question.
    /// Never a system disk: the safety filter stays in charge
    #[arg(long)]
    pub(super) force: bool,
}

impl RepairArgs {
    pub(crate) fn touches_real_disk(&self) -> bool {
        !self.common.simulate
    }
}
