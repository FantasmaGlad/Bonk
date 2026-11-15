use crate::config::ConfigAction;
use crate::errors::ExplainArgs;
use crate::logging::LogArgs;
use crate::schema::SchemaArgs;
use crate::transfer::{RepairArgs, VerifyArgs, WriteArgs};
use clap::{Parser, Subcommand};

use crate::*;

/// Without a command, the window opens.
#[derive(Debug, Parser)]
// Without it clap reuses the doc comment as long help, and `--help` would show a developer note instead of
// the tool description.
#[command(
    name = "bonk",
    version,
    about = "Create bootable media (USB drives and external disks)",
    long_about = None,
    // Read by `parse` before parsing: no field holds it.
    arg(
        clap::Arg::new("no_color")
            .long("no-color")
            .global(true)
            .action(clap::ArgAction::SetTrue)
            .help("No color in the help and usage errors (`NO_COLOR` has the same effect)"),
    ),
)]
pub(crate) struct Cli {
    #[command(flatten)]
    pub(crate) log: LogArgs,

    #[command(flatten)]
    pub(crate) global: fixture::GlobalArgs,

    /// Never open the window: without a command, print the help and fail, for scripts
    #[arg(long)]
    pub(crate) no_gui: bool,

    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, clap::Args)]
pub(crate) struct GuiArgs {
    /// Open with this image of the signed catalog chosen (`bonk catalog list`); nothing is written
    #[arg(long, value_name = "ID", conflicts_with = "link")]
    pub(crate) image: Option<String>,
    /// Same, from a `bonk://install/<ID>` link: what the system passes when such a link is opened
    #[arg(long, value_name = "URL")]
    pub(crate) link: Option<String>,
}

impl GuiArgs {
    /// The identifier is checked here in both forms: the window only ever receives a catalog name.
    pub(crate) fn image(&self) -> Result<Option<String>, bonk_catalog::link::LinkError> {
        use bonk_catalog::link::{LinkError, image_id, is_image_id};
        match (&self.image, &self.link) {
            (Some(id), _) if is_image_id(id) => Ok(Some(id.clone())),
            (Some(_), _) => Err(LinkError::InvalidImageId),
            (None, Some(link)) => image_id(link).map(|id| Some(id.to_owned())),
            (None, None) => Ok(None),
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Disks: list and detail
    Disk {
        #[command(subcommand)]
        action: DiskAction,
    },
    /// Images offered by the signed catalog
    Catalog {
        #[command(subcommand)]
        action: catalog::CatalogAction,
    },
    /// Downloaded images: list and cleanup
    Cache {
        #[command(subcommand)]
        action: cache::CacheAction,
    },
    /// List the disks Bonk can write (shortcut for `disk list`)
    List(list::ListArgs),
    /// Detail of a disk (shortcut for `disk info`)
    Info(info::InfoArgs),
    /// Write an image to a disk, then verify it (`--simulate`: to an image file)
    Write(WriteArgs),
    /// Compare a disk to an image, writing nothing
    Verify(VerifyArgs),
    /// Repair the GPT of a disk or an image
    RepairGpt(RepairArgs),
    /// Return a disk to storage use: table, partition, filesystem
    Format(format::FormatArgs),
    /// Disk carrying several ISO images: prepare, list
    Vault {
        #[command(subcommand)]
        action: vault::VaultAction,
    },
    /// What this host allows, and why not the rest
    Capabilities(format::CapabilitiesArgs),
    /// Support report as JSON: system, rendering, disks and verdicts, secrets masked
    Diagnose(diagnose::DiagnoseArgs),
    /// Low-level disk view, read-only: system sources, step-by-step filter, partition tables
    Debug {
        #[command(subcommand)]
        action: debug::DebugAction,
    },
    /// Explain an error code (`bonk explain E1003`) or list all codes
    Explain(ExplainArgs),
    /// Persistent settings: list, get, set, reset
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// JSON Schema of a command's `--json` output
    Schema(SchemaArgs),
    /// Shell completions: bash, zsh, fish, powershell, elvish
    Completions {
        /// Target shell
        shell: clap_complete::Shell,
    },
    /// Headless service: drive Bonk with JSON on standard input and output, without a window
    Serve,
    /// Open the window (like `bonk` alone)
    Gui(GuiArgs),
    /// Windows: the elevated side of an authorization, started by Bonk itself
    #[command(hide = true)]
    ElevatedHelper {
        #[arg(long)]
        client_pid: u32,
        #[arg(long)]
        pipe: String,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum DiskAction {
    /// List the disks Bonk can write (read-only)
    List(list::ListArgs),
    /// Detail of a disk (read-only)
    Info(info::InfoArgs),
    /// Write an image to a disk, then verify it
    Write(WriteArgs),
    /// Compare a disk to an image, writing nothing
    Verify(VerifyArgs),
    /// Repair the GPT of a disk
    RepairGpt(RepairArgs),
}
