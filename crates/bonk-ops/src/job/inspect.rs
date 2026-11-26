use std::path::Path;

use bonk_core::DEFAULT_BLOCK_SIZE;
use bonk_core::analysis::ImageReport;
use bonk_core::codec::Format;
use bonk_core::efi::{self, Loader};
use bonk_core::format::table::TableKind;
use bonk_core::hybrid::{NotHybrid, hybrid_mbr};
use bonk_core::route::{self, Refusal, Way, WriteMode};
use bonk_core::vault::{VaultFamily, VaultMatch, vault_match};
use schemars::JsonSchema;
use serde::Serialize;

use super::extract::{self, Extraction, WindowsImage};
use super::source::ImageFile;
use super::types::{JobError, SCHEMA_VERSION, SourceInfo, fail};
use super::write::analyze_source;
use crate::errors::Code;

/// What Bonk reads in an image before any write: its structure, the route `bonk write` takes and the UEFI loaders.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ImageInspection {
    pub schema_version: u32,
    pub source: SourceInfo,
    pub image: ImageReport,
    /// Route of `bonk write` in automatic mode; null when it refuses the image.
    pub route: Option<Way>,
    /// Why `bonk write` refuses the image in automatic mode.
    pub refusal: Option<String>,
    /// `--mode hybrid` writes this image, which then starts in UEFI only.
    pub hybrid_possible: bool,
    /// Windows drive the image would give, with the default MBR table.
    pub extraction: Option<Extraction>,
    /// Removable-media UEFI loaders of the image and who signed them.
    pub efi_loaders: Vec<Loader>,
    /// The vault family that starts this ISO; null when no family does, or for a compressed image.
    pub vault: Option<VaultMatch>,
}

/// # Errors
/// Unreadable or undecodable image.
pub fn inspect(path: &Path, families: &[VaultFamily]) -> Result<ImageInspection, JobError> {
    inspect_image(&ImageFile::open(path)?, families)
}

/// # Errors
/// Unreadable or undecodable image.
pub fn inspect_image(
    file: &ImageFile,
    families: &[VaultFamily],
) -> Result<ImageInspection, JobError> {
    let (_, source) = file.decoded()?;
    let (image, head) = analyze_source(file, Path::new(&source.path), source.size_bytes)?;
    let compressed = source.format != Format::Raw;
    let (windows, unextractable) = if route::looks_for_windows(&image, WriteMode::Auto, compressed)
    {
        windows_of(file)?
    } else {
        (None, None)
    };
    let (route, refusal, extraction) = planned_route(&image, compressed, windows);
    let refusal = unextractable.or_else(|| refusal.as_ref().map(Refusal::to_string));
    let route = route.filter(|_| refusal.is_none());
    let (hybrid_possible, efi_loaders) = if compressed {
        (
            hybrid_mbr(head.as_slice()).is_ok(),
            efi::loaders(head.as_slice(), &head),
        )
    } else {
        (hybrid_mbr(file).is_ok(), efi::loaders(file, &head))
    };
    let vault = if compressed {
        None
    } else {
        vault_match(file, file.size(), families)
            .map_err(|e| fail(Code::SourceUnreadable, format!("{}: {e}", source.path)))?
    };
    Ok(ImageInspection {
        schema_version: SCHEMA_VERSION,
        source,
        image,
        route,
        hybrid_possible: hybrid_possible && route != Some(Way::Raw),
        refusal,
        extraction,
        efi_loaders,
        vault,
    })
}

// A Windows installation Bonk cannot lay out on FAT32 is refused by `bonk write`: the report says why instead of
// failing. An unreadable image stays an error.
fn windows_of(file: &ImageFile) -> Result<(Option<WindowsImage>, Option<String>), JobError> {
    match extract::detect(file) {
        Ok(windows) => Ok((windows, None)),
        Err(error) if error.code == Code::SourceFormatUnsupported => {
            Ok((None, Some(error.message)))
        }
        Err(error) => Err(error),
    }
}

fn planned_route(
    image: &ImageReport,
    compressed: bool,
    windows: Option<WindowsImage>,
) -> (Option<Way>, Option<Refusal>, Option<Extraction>) {
    let planned = route::plan(image, WriteMode::Auto, compressed, windows, || {
        Err::<(), _>(NotHybrid::NoEfiImage)
    });
    match planned {
        Ok(planned) => {
            let way = planned.route.way();
            let extraction = match planned.route {
                route::Route::Extract(windows) => {
                    Some(windows.describe(TableKind::WINDOWS_DEFAULT, DEFAULT_BLOCK_SIZE))
                }
                _ => None,
            };
            (Some(way), None, extraction)
        }
        Err(refusal) => (None, Some(refusal), None),
    }
}
