use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;
use std::path::PathBuf;

use super::download;

#[derive(Debug, Subcommand)]
pub(crate) enum CatalogAction {
    /// List the images offered, newest version of each system first
    List(ListArgs),
    /// Download an image into the cache and check its hash, without writing it
    Download(download::DownloadArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ListArgs {
    /// Keep one system only (`ubuntu`)
    #[arg(long, value_name = "ID")]
    pub(super) family: Option<String>,
    /// Keep the images whose system or image name holds every word, any case or accent (`kubuntu`)
    #[arg(long, value_name = "TEXT")]
    pub(super) search: Option<String>,
    /// Keep one usage category of the catalog (`office`, `server`)
    #[arg(long = "use", value_name = "ID")]
    pub(super) uses: Option<String>,
    /// Keep one base of the catalog (`debian`, `arch`)
    #[arg(long, value_name = "BASE")]
    pub(super) category: Option<String>,
    /// Keep one desktop (`gnome`, `kde`, `none`)
    #[arg(long, value_name = "ID")]
    pub(super) desktop: Option<String>,
    /// Keep one medium (`usb`, `sd`, `optical`)
    #[arg(long, value_name = "ID")]
    pub(super) media: Option<String>,
    /// Keep one architecture, as the catalog names it (`x86_64`, `aarch64`, `i686`, `riscv64`)
    #[arg(long, value_name = "ARCH")]
    pub(super) arch: Option<String>,
    /// Keep one life cycle
    #[arg(long, value_enum)]
    pub(super) channel: Option<ChannelArg>,
    /// Order of the list
    #[arg(long, value_enum, default_value_t = SortBy::System)]
    pub(super) sort: SortBy,
    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL")]
    pub(super) catalog_url: Option<String>,
    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    pub(super) cache_dir: Option<PathBuf>,
    /// Result as JSON
    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub(crate) enum ChannelArg {
    Lts,
    Stable,
    EndOfLife,
    Archive,
}

/// The service gets the same names as the command line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SortBy {
    /// By system, newest version first
    #[default]
    System,
    /// Most recently released first
    Released,
    /// Alphabetical
    Name,
    /// Smallest download first
    Size,
}
