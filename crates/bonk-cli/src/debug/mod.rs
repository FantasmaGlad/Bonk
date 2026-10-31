//! Sous-commandes `bonk debug ...` : le bas niveau des disques, **en lecture seule par construction**.
//! Aucune fonction de ce module n'écrit sur un disque ou une image : elles ne touchent que
//! `BlockDevice::read_at` et des lectures de fichiers système.

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

/// Taille d'un dump par défaut.
const HEXDUMP_DEFAULT: u64 = 4096;

/// Plafond d'un dump.
const HEXDUMP_MAX: u64 = 1 << 20;

#[derive(Debug, Subcommand)]
pub(crate) enum DebugAction {
    /// Chaque source système lue par l'énumération, avec sa durée (et son contenu avec --content)
    Sources(SourcesArgs),
    /// Rejoue le filtre de sécurité sur un disque, règle par règle, avec les faits lus
    Filter(FilterArgs),
    /// Décode MBR, GPT principal et de secours d'une image, et liste les anomalies
    Layout(LayoutArgs),
    /// Octets bruts d'une image, par lignes de 16
    Hexdump(HexdumpArgs),
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct SourcesArgs {
    /// Sortie JSON (contient toujours le contenu des sources)
    #[arg(long)]
    json: bool,
    /// Affiche aussi le contenu de chaque source, tel que lu
    #[arg(long)]
    content: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct FilterArgs {
    /// Identifiant stable du disque ou son chemin
    target: String,
    /// Sortie JSON
    #[arg(long)]
    json: bool,
    /// Évalue avec les disques internes non système proposés
    #[arg(long)]
    show_internal: bool,
    /// Évalue en mode test : disques virtuels autorisés
    #[arg(long)]
    allow_virtual: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct LayoutArgs {
    /// Fichier image ou disque (identifiant ou /dev/...) à décoder ; un disque demande l'autorisation du système
    image: PathBuf,
    /// Sortie JSON
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
#[command(long_about = None)]
pub(crate) struct HexdumpArgs {
    /// Fichier image ou disque (identifiant ou /dev/...) à lire ; un disque demande l'autorisation du système
    image: PathBuf,
    /// Décalage de départ en octets (décimal ou 0x...)
    #[arg(long, default_value = "0", value_parser = parse_number)]
    offset: u64,
    /// Nombre d'octets (décimal ou 0x...), 4096 par défaut, 1048576 au plus
    #[arg(long, default_value_t = HEXDUMP_DEFAULT, value_parser = parse_number)]
    len: u64,
}

/// Nombre décimal ou hexadécimal (`0x1BE`).
pub(crate) fn parse_number(text: &str) -> Result<u64, String> {
    let parsed = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => text.parse(),
    };
    parsed.map_err(|_| format!("nombre invalide : « {text} » (décimal ou 0x...)"))
}

/// Ce que `debug` a à dire à l'appelant en plus de la sortie.
pub(crate) enum Outcome {
    Done,
    /// Échec expliqué par un message pour la sortie d'erreur.
    Failed(Code, String),
}
