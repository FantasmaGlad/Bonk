//! Obtenir une image depuis le réseau : téléchargement reprenable, fichiers d'empreintes, manifeste signé.
//! Ne dépend d'aucun autre crate du workspace.

mod download;
pub mod manifest;
mod sums;

pub use download::{
    Advance, DownloadError, Hooks, SILENCE_LIMIT, download, fetch_text, file_name, is_url,
};
pub use sums::find_sha256;
