use std::path::PathBuf;

use bonk_core::format::table::TableKind;

use crate::model::{Model, WriteState};

/// Where the image to write comes from. Each fills the write panel the same way, so a new source needs no
/// change to the views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageSource {
    /// A signed catalog entry: its address and official hash.
    Catalog { url: String, sha256: String },
    /// A file chosen or dropped by the user; Bonk analyses it and the user may give its hash.
    File(PathBuf),
    /// An image already downloaded, kept under the official hash it was checked against.
    Kept { path: PathBuf, sha256: String },
}

impl ImageSource {
    /// The menu only names catalog images: any other source shows itself in the options.
    #[must_use]
    pub fn shows_options(&self) -> bool {
        !matches!(self, Self::Catalog { .. })
    }
}

impl Model {
    /// Replaces the image of the open write panel; without a panel, nothing changes. A known hash also
    /// gives the mode back to Bonk; a chosen file keeps the hash and mode the user set.
    pub fn fill_draft(&mut self, source: &ImageSource) {
        let WriteState::Draft {
            image,
            hash,
            raw,
            mbr,
            ..
        } = &mut self.write
        else {
            return;
        };
        let (shown, known) = match source {
            ImageSource::Catalog { url, sha256 } => (url.clone(), Some(sha256)),
            ImageSource::File(path) => (path.display().to_string(), None),
            ImageSource::Kept { path, sha256 } => (path.display().to_string(), Some(sha256)),
        };
        *image = shown;
        if let Some(sha256) = known {
            sha256.clone_into(hash);
            *raw = false;
            *mbr = TableKind::WINDOWS_DEFAULT == TableKind::Mbr;
        }
        if source.shows_options() {
            self.advanced = true;
        }
    }
}
