use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Serialize;

/// The GRUB written to the boot partition, as the signed catalog names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GrubInfo {
    pub version: String,
    pub build: String,
    /// SHA-256 of each EFI image written, by processor (`x86_64`, `aarch64`).
    pub efi: BTreeMap<String, String>,
    /// SHA-256 of `boot.img` and `core.img`, GRUB for BIOS machines; empty when the catalog offers none.
    pub bios: BTreeMap<String, String>,
}
