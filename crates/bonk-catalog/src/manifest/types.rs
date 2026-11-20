use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A system as the choice screen shows it: its versions are its images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Family {
    /// Stable identifier: `ubuntu`.
    pub id: String,
    pub name: String,
    /// Technical base, one of the catalog's `taxonomy.bases`: `debian`. Absent from older catalogs.
    #[serde(default)]
    pub category: Option<String>,
    /// Logo, relative to the manifest. Its hash, covered by the signature, keeps it from being replaced.
    pub icon: String,
    pub icon_sha256: String,
    /// The system a flavour derives from: `ubuntu` for Kubuntu.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    /// Official address the logo comes from.
    #[serde(default)]
    pub icon_source: Option<String>,
    /// Usage categories, among the catalog's `taxonomy.uses`: `office`, `server`.
    #[serde(default)]
    pub uses: Vec<String>,
    /// Minimum memory the publisher states, in MiB, and the page it is read on.
    #[serde(default)]
    pub ram_mb: Option<u32>,
    #[serde(default)]
    pub ram_source: Option<String>,
}

/// A closed name of the catalog with the labels interfaces show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Term {
    pub id: String,
    pub en: String,
    pub fr: String,
}

/// The usage categories and technical bases, in menu order: a new one needs no Bonk release.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Taxonomy {
    #[serde(default)]
    pub uses: Vec<Term>,
    #[serde(default)]
    pub bases: Vec<Term>,
}

/// Life cycle of a version: what marks an old version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Long-term maintained by the publisher (Ubuntu 26.04 LTS).
    Lts,
    /// Current version.
    Stable,
    /// Still offered, no longer maintained by the publisher: the interface says so.
    EndOfLife,
    /// An older version, served by the publisher's own archive.
    Archive,
}

/// An offered image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Image {
    /// Stable identifier: `debian-13-netinst-x86_64`.
    pub id: String,
    /// Identifier of the [`Family`].
    pub family: String,
    pub name: String,
    pub version: String,
    pub channel: Channel,
    /// Release date `YYYY-MM-DD`, to sort versions.
    pub released: String,
    #[serde(default)]
    pub edition: Option<String>,
    pub arch: String,
    pub homepage: String,
    /// What to know before choosing it: 90-day evaluation, CD only.
    #[serde(default)]
    pub note: Option<String>,
    /// The file boots once written to a USB drive. False: CD or virtual machine only.
    #[serde(default = "boots_from_usb")]
    pub usb_boot: bool,
    /// Other addresses of the same file: same hash, same size. `.onion` ones may be plain http.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Desktop installed: `none` without one, `choice` picked during installation.
    #[serde(default)]
    pub desktop: Option<String>,
    /// `live`, `installer`, `netinst`, `preinstalled` or `rescue`.
    #[serde(default)]
    pub kind: Option<String>,
    /// `usb`, `sd` (ARM boards) or `optical` (CD or virtual machine only).
    #[serde(default)]
    pub media: Option<String>,
    /// Memory the publisher states for this variant, in MiB, when it differs from the system's.
    #[serde(default)]
    pub ram_mb: Option<u32>,
    #[serde(default)]
    pub ram_source: Option<String>,
    /// Starts from a Bonk vault (`bonk vault add`), checked by a boot, even when it does not start written to a
    /// drive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vault: bool,
    /// Starts from a drive written with `--mode hybrid` (an MBR naming its EFI boot image), checked by a boot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hybrid: bool,
    /// Name to save the file under, when the address does not end with it (a share link).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(flatten)]
    pub source: ImageSource,
}

impl Image {
    /// `None` for an official page, or a name that would leave the cache directory.
    #[must_use]
    pub fn download_name(&self) -> Option<&str> {
        match &self.source {
            ImageSource::Download { url, .. } => {
                crate::download_name(url, self.file_name.as_deref())
            }
            ImageSource::OfficialPage { .. } => None,
        }
    }
}

pub(super) fn boots_from_usb() -> bool {
    true
}

/// Where to get the image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ImageSource {
    /// Official link, hash read from the publisher.
    Download {
        url: String,
        sha256: String,
        size_bytes: u64,
    },
    /// Publisher without a stable link (Windows): the user downloads it.
    OfficialPage { official_page: String },
}

/// `24.04.5` sorts after `26.04.1` even if released later, and `9` before `10`.
#[must_use]
pub fn version_numbers(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// A system in the report of `bonk catalog list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FamilyEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub uses: Vec<String>,
    /// Logo checked by its hash, in the local cache; absent if it could not be read.
    #[serde(default)]
    pub icon_path: Option<std::path::PathBuf>,
}

/// Report of `bonk catalog list --json`: what interfaces read, one type for both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CatalogReport {
    pub schema_version: u32,
    pub sequence: u64,
    pub expires_unix: u64,
    /// Saved copy, because the network was missing.
    pub saved_copy: bool,
    pub families: Vec<FamilyEntry>,
    pub images: Vec<Image>,
    /// Absent from older catalogs: interfaces fall back on their own copy.
    #[serde(default)]
    pub taxonomy: Option<Taxonomy>,
}

/// The manifest, once verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    pub schema_version: u32,
    /// Grows at each publication: a mirror cannot serve an older manifest to someone who saw a newer one.
    pub sequence: u64,
    pub expires_unix: u64,
    pub families: Vec<Family>,
    pub images: Vec<Image>,
    /// Boot loader of the vault. Absent from older catalogs.
    #[serde(default)]
    pub grub: Option<Grub>,
    #[serde(default)]
    pub taxonomy: Option<Taxonomy>,
}

/// A file published next to the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    /// Relative to the manifest.
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// GRUB built from the signed GNU archive, with its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Grub {
    pub version: String,
    pub released: String,
    /// Changes when the build changes, even for the same version.
    pub build: String,
    /// One standalone EFI image per architecture: `x86_64`, later `aarch64` and `riscv64`.
    pub efi: BTreeMap<String, Asset>,
    pub source: Asset,
    pub source_signature: Asset,
}
