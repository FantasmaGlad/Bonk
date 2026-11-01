//! Ce que toutes les interfaces de Bonk partagent : état, règles d'écran et textes de l'écriture. Sans Iced ni
//! système : la fenêtre de bureau et l'application Android dessinent le même modèle et accomplissent ses effets.

pub mod catalog;
pub mod command;
pub mod errors;
pub mod event;
pub mod model;
mod navigation;
mod update;

pub use errors::french_title;
