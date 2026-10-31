//! Obtenir une image depuis le réseau : téléchargement reprenable, fichiers d'empreintes. Le manifeste signé
//! viendra ici. Ne dépend d'aucun autre crate du workspace.

mod download;
mod sums;

pub use download::{
    Advance, DownloadError, Hooks, SILENCE_LIMIT, download, fetch_text, file_name, is_url,
};
pub use sums::find_sha256;
