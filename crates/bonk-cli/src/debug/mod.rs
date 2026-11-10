mod filter;
mod hexdump;
mod layout;
mod sources;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::errors::Code;

pub(crate) use filter::{filter, filter_value, schema_filter};
pub(crate) use hexdump::hexdump;
pub(crate) use layout::{layout, schema_layout};
pub(crate) use sources::{schema_sources, sources};

const JSON_SCHEMA_VERSION: u32 = 1;

const HEXDUMP_DEFAULT: u64 = 4096;

const HEXDUMP_MAX: u64 = 1 << 20;

#[derive(Debug, Subcommand)]
pub(crate) enum DebugAction {
    /// Each system source read by the enumeration, with its duration (and its content with --content)
    Sources(SourcesArgs),
    /// Replay the safety filter on a disk, rule by rule, with the facts read
    Filter(FilterArgs),
    /// Decode the MBR, primary and backup GPT of an image, and list the anomalies
    Layout(LayoutArgs),
    /// Raw bytes of an image, in lines of 16
    Hexdump(HexdumpArgs),
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct SourcesArgs {
    /// JSON output (always contains the source content)
    #[arg(long)]
    json: bool,
    /// Also show the content of each source, as read
    #[arg(long)]
    content: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct FilterArgs {
    /// Stable identifier of the disk or its path
    target: String,
    /// JSON output
    #[arg(long)]
    json: bool,
    /// Evaluate with internal non-system disks offered
    #[arg(long)]
    show_internal: bool,
    /// Evaluate in test mode: virtual disks allowed
    #[arg(long)]
    allow_virtual: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct LayoutArgs {
    /// Image file or disk (identifier or /dev/...) to decode; a disk asks for system authorization
    image: PathBuf,
    /// JSON output
    #[arg(long)]
    json: bool,
    /// Log each block access (offset, duration, checksum); `=data` adds the
    /// first bytes, for an image file only
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "metadata", require_equals = true)]
    pub(super) trace_io: Option<crate::transfer::TraceIoArg>,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct HexdumpArgs {
    /// Image file or disk (identifier or /dev/...) to read; a disk asks for system authorization
    image: PathBuf,
    /// Start offset in bytes (decimal or 0x...)
    #[arg(long, default_value = "0", value_parser = parse_number)]
    offset: u64,
    /// Number of bytes (decimal or 0x...), 4096 by default, at most 1048576
    #[arg(long, default_value_t = HEXDUMP_DEFAULT, value_parser = parse_number)]
    len: u64,
    /// Log each block access (offset, duration, checksum); `=data` adds the
    /// first bytes, for an image file only
    #[arg(long, value_enum, num_args = 0..=1, default_missing_value = "metadata", require_equals = true)]
    pub(super) trace_io: Option<crate::transfer::TraceIoArg>,
}

/// Decimal or hexadecimal (`0x1BE`).
pub(crate) fn parse_number(text: &str) -> Result<u64, String> {
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("invalid number: '{text}' (decimal or 0x...)"))
}

pub(crate) enum Outcome {
    Done,
    Failed(Code, String),
}

impl LayoutArgs {
    pub(crate) fn reads_image_file(&self) -> bool {
        self.image.is_file()
    }
}

impl HexdumpArgs {
    pub(crate) fn reads_image_file(&self) -> bool {
        self.image.is_file()
    }
}
