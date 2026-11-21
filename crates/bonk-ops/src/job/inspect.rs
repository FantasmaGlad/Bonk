use std::path::Path;

use bonk_core::DEFAULT_BLOCK_SIZE;
use bonk_core::analysis::ImageReport;
use bonk_core::codec::Format;
use bonk_core::efi::{self, Loader};
use bonk_core::format::table::TableKind;
use bonk_core::hybrid::{NotHybrid, hybrid_mbr};
use bonk_core::route::{self, Refusal, Way, WriteMode};
use schemars::JsonSchema;
use serde::Serialize;

use super::extract::{self, Extraction};
use super::source::ImageFile;
use super::types::{JobError, SCHEMA_VERSION, SourceInfo};
use super::write::analyze_source;

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
}

/// # Errors
/// Unreadable or undecodable image.
pub fn inspect(path: &Path) -> Result<ImageInspection, JobError> {
    let file = ImageFile::open(path)?;
    let (_, source) = file.decoded()?;
    let (image, head) = analyze_source(&file, path, source.size_bytes)?;
    let compressed = source.format != Format::Raw;
    let windows = if route::looks_for_windows(&image, WriteMode::Auto, compressed) {
        extract::detect(&file)?
    } else {
        None
    };
    let planned = route::plan(&image, WriteMode::Auto, compressed, windows, || {
        Err::<(), _>(NotHybrid::NoEfiImage)
    });
    let (route, refusal, extraction) = match planned {
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
    };
    let (hybrid_possible, efi_loaders) = if compressed {
        (
            hybrid_mbr(head.as_slice()).is_ok(),
            efi::loaders(head.as_slice(), &head),
        )
    } else {
        (hybrid_mbr(&file).is_ok(), efi::loaders(&file, &head))
    };
    Ok(ImageInspection {
        schema_version: SCHEMA_VERSION,
        source,
        image,
        route,
        hybrid_possible: hybrid_possible && route != Some(Way::Raw),
        refusal: refusal.as_ref().map(Refusal::to_string),
        extraction,
        efi_loaders,
    })
}
