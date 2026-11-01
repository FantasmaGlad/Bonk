//! `bonk catalog` : les images proposées, lues dans le catalogue signé, et `--image`, qui en écrit une.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_catalog::manifest::{self, Catalog, Channel, Image, ImageSource, Loaded, TRUSTED_KEYS};
use bonk_hal::{Lang, format_size};
use clap::{Args, Subcommand};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::{Code, report};
use bonk_ops::job::{self, JobError};

/// Adresse du catalogue, jusqu'au dossier de la version du format.
const DEFAULT_URL: &str = "https://catalog.bonk-os.app/v1/";

#[derive(Debug, Subcommand)]
pub(crate) enum CatalogAction {
    /// List the images offered, newest version of each system first
    List(ListArgs),
}

#[derive(Debug, Args)]
pub(crate) struct ListArgs {
    /// Keep one system only (`ubuntu`)
    #[arg(long, value_name = "ID")]
    family: Option<String>,
    /// Catalog address (`BONK_CATALOG_URL`); its signature is checked wherever it comes from
    #[arg(long, value_name = "URL")]
    catalog_url: Option<String>,
    /// Download directory; by default the user's cache directory
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,
    /// Result as JSON
    #[arg(long)]
    json: bool,
}

/// L'adresse donnée, sinon la variable, sinon celle de Bonk ; toujours terminée par `/`.
fn base_url(given: Option<&str>, variable: Option<String>) -> String {
    let url = given
        .map(str::to_owned)
        .or(variable)
        .unwrap_or_else(|| DEFAULT_URL.to_owned());
    if url.ends_with('/') {
        url
    } else {
        format!("{url}/")
    }
}

fn refused(message: impl std::fmt::Display) -> JobError {
    JobError {
        code: Code::CatalogRefused,
        message: message.to_string(),
    }
}

/// Lit le catalogue : réseau, signature, copie gardée si le réseau manque.
pub(crate) fn load(
    url: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<(Loaded, PathBuf), JobError> {
    let base = base_url(url, std::env::var("BONK_CATALOG_URL").ok());
    let dir = crate::cache::directory(cache_dir)
        .ok_or_else(|| refused("no known cache directory: give it with --cache-dir"))?
        .join("catalog");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let fetch = |address: &str| bonk_catalog::fetch_bytes(address).map_err(|e| e.to_string());
    let loaded = manifest::load(&base, &dir, now, &TRUSTED_KEYS, &fetch).map_err(refused)?;
    if loaded.saved_copy {
        log::warn!(target: "bonk::catalog", "catalog not reachable: using the saved copy");
    }
    Ok((loaded, dir))
}

/// Ce qu'il faut pour écrire une image du catalogue : son adresse et son empreinte.
pub(crate) fn resolve_image(catalog: &Catalog, id: &str) -> Result<(PathBuf, String), JobError> {
    let image = catalog
        .images
        .iter()
        .find(|image| image.id == id)
        .ok_or_else(|| JobError {
            code: Code::InvalidArgument,
            message: format!("unknown image: '{id}' (images: bonk catalog list)"),
        })?;
    match &image.source {
        ImageSource::Download { url, sha256, .. } => Ok((PathBuf::from(url), sha256.clone())),
        ImageSource::OfficialPage { official_page } => Err(JobError {
            code: Code::InvalidArgument,
            message: format!(
                "{} {} has no direct link: download it from {official_page}, then give the file",
                image.name, image.version
            ),
        }),
    }
}

/// `--image` : l'adresse et l'empreinte de l'image demandée.
pub(crate) fn image_source(
    id: &str,
    url: Option<&str>,
    cache_dir: Option<&Path>,
) -> Result<(PathBuf, String), JobError> {
    let (loaded, _) = load(url, cache_dir)?;
    resolve_image(&loaded.catalog, id)
}

/// Les nombres d'une version : `24.04.5` passe après `26.04.1`, même sorti plus tard, et `9` avant `10`.
fn version_numbers(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// Les systèmes dans l'ordre du catalogue, la version la plus récente d'abord.
pub(crate) fn sorted<'a>(catalog: &'a Catalog, family: Option<&str>) -> Vec<&'a Image> {
    let rank = |image: &Image| {
        catalog
            .families
            .iter()
            .position(|f| f.id == image.family)
            .unwrap_or(usize::MAX)
    };
    let mut images: Vec<&Image> = catalog
        .images
        .iter()
        .filter(|image| family.is_none_or(|wanted| image.family == wanted))
        .collect();
    images.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| version_numbers(&b.version).cmp(&version_numbers(&a.version)))
            .then_with(|| a.id.cmp(&b.id))
    });
    images
}

fn channel_label(channel: Channel) -> &'static str {
    match channel {
        Channel::Lts => "LTS",
        Channel::Stable => "stable",
        Channel::EndOfLife => "END OF LIFE",
    }
}

#[derive(Serialize, JsonSchema)]
struct FamilyEntry {
    id: String,
    name: String,
    /// Logo contrôlé par son empreinte, dans le cache local ; absent s'il n'a pas pu être lu.
    icon_path: Option<PathBuf>,
}

#[derive(Serialize, JsonSchema)]
struct Report {
    schema_version: u32,
    sequence: u64,
    expires_unix: u64,
    /// Copie gardée, le réseau ayant manqué.
    saved_copy: bool,
    families: Vec<FamilyEntry>,
    images: Vec<Image>,
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Report)).unwrap_or_default()
}

/// Le rapport JSON, partagé avec le service.
pub(crate) fn report_value(
    url: Option<&str>,
    cache_dir: Option<&Path>,
    family: Option<&str>,
) -> Result<serde_json::Value, JobError> {
    let (loaded, dir) = load(url, cache_dir)?;
    let base = base_url(url, std::env::var("BONK_CATALOG_URL").ok());
    let fetch = |address: &str| bonk_catalog::fetch_bytes(address).map_err(|e| e.to_string());
    let catalog = &loaded.catalog;
    let report = Report {
        schema_version: job::SCHEMA_VERSION,
        sequence: catalog.sequence,
        expires_unix: catalog.expires_unix,
        saved_copy: loaded.saved_copy,
        families: catalog
            .families
            .iter()
            .map(|f| FamilyEntry {
                id: f.id.clone(),
                name: f.name.clone(),
                icon_path: manifest::icon_path(&base, &dir, f, &fetch),
            })
            .collect(),
        images: sorted(catalog, family).into_iter().cloned().collect(),
    };
    serde_json::to_value(report).map_err(refused)
}

fn write_table(catalog: &Catalog, family: Option<&str>, out: &mut dyn Write) -> io::Result<()> {
    let rows: Vec<[String; 5]> = sorted(catalog, family)
        .into_iter()
        .map(|image| {
            let size = match &image.source {
                ImageSource::Download { size_bytes, .. } => format_size(*size_bytes, Lang::En),
                ImageSource::OfficialPage { .. } => "official page".to_owned(),
            };
            [
                image.id.clone(),
                format!("{} {}", image.name, image.version),
                channel_label(image.channel).to_owned(),
                image.released.clone(),
                size,
            ]
        })
        .collect();
    let header = ["ID", "SYSTEM", "CHANNEL", "RELEASED", "SIZE"].map(str::to_owned);
    let mut widths = [0; 5];
    for row in std::iter::once(&header).chain(&rows) {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.len());
        }
    }
    for row in std::iter::once(&header).chain(&rows) {
        let line: Vec<String> = row
            .iter()
            .zip(widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        writeln!(out, "{}", line.join("  ").trim_end())?;
    }
    Ok(())
}

pub(crate) fn run(action: &CatalogAction, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let CatalogAction::List(args) = action;
    let url = args.catalog_url.as_deref();
    let cache = args.cache_dir.as_deref();
    let written = if args.json {
        report_value(url, cache, args.family.as_deref()).map(|value| {
            serde_json::to_writer_pretty(&mut *out, &value)
                .map_err(io::Error::from)
                .and_then(|()| writeln!(out))
        })
    } else {
        load(url, cache)
            .map(|(loaded, _)| write_table(&loaded.catalog, args.family.as_deref(), out))
    };
    match written {
        Ok(Ok(())) => 0,
        Ok(Err(error)) => report(err, Code::OutputFailed, error),
        Err(error) => report(err, error.code, error.message),
    }
}

#[cfg(test)]
mod tests {
    use bonk_catalog::manifest::Family;

    use super::*;

    fn image(id: &str, family: &str, released: &str, source: ImageSource) -> Image {
        Image {
            id: id.to_owned(),
            family: family.to_owned(),
            name: family.to_owned(),
            version: "1".to_owned(),
            channel: Channel::Lts,
            released: released.to_owned(),
            edition: None,
            arch: "x86_64".to_owned(),
            homepage: "https://h/".to_owned(),
            source,
        }
    }

    fn download() -> ImageSource {
        ImageSource::Download {
            url: "https://h/x.iso".to_owned(),
            sha256: "a".repeat(64),
            size_bytes: 2_000_000_000,
        }
    }

    fn catalog() -> Catalog {
        let family = |id: &str| Family {
            id: id.to_owned(),
            name: id.to_owned(),
            icon: format!("icons/{id}.svg"),
            icon_sha256: "b".repeat(64),
        };
        Catalog {
            schema_version: 1,
            sequence: 1,
            expires_unix: 2,
            families: vec![family("ubuntu"), family("windows")],
            images: vec![
                image(
                    "w",
                    "windows",
                    "2021-10-05",
                    ImageSource::OfficialPage {
                        official_page: "https://p/".to_owned(),
                    },
                ),
                image("u22", "ubuntu", "2024-09-11", download()),
                image("u26", "ubuntu", "2026-08-26", download()),
            ],
        }
    }

    #[test]
    fn systemes_dans_l_ordre_et_versions_recentes_d_abord() {
        let catalog = catalog();
        let ids: Vec<&str> = sorted(&catalog, None)
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["u22", "u26", "w"]);
        assert_eq!(sorted(&catalog, Some("windows")).len(), 1);
        // Une version plus ancienne sortie plus tard reste après la plus récente.
        let mut tardive = catalog.clone();
        tardive.images[1].version = "24.04.5".to_owned();
        tardive.images[1].released = "2027-01-01".to_owned();
        tardive.images[2].version = "26.04.1".to_owned();
        let ids: Vec<&str> = sorted(&tardive, Some("ubuntu"))
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["u26", "u22"]);
    }

    #[test]
    fn une_image_se_resout_ou_se_refuse_en_disant_pourquoi() -> Result<(), JobError> {
        let catalog = catalog();
        let (url, sha) = resolve_image(&catalog, "u26")?;
        assert_eq!((url.to_str(), sha.len()), (Some("https://h/x.iso"), 64));
        let inconnue = resolve_image(&catalog, "absente").err().map(|e| e.code);
        assert_eq!(inconnue, Some(Code::InvalidArgument));
        let page = resolve_image(&catalog, "w")
            .err()
            .map(|e| e.message)
            .unwrap_or_default();
        assert!(page.contains("https://p/"), "{page}");
        Ok(())
    }

    #[test]
    fn adresse_donnee_puis_variable_puis_defaut() {
        assert_eq!(
            base_url(Some("https://a/b"), Some("https://c/".into())),
            "https://a/b/"
        );
        assert_eq!(base_url(None, Some("https://c".into())), "https://c/");
        assert_eq!(base_url(None, None), DEFAULT_URL);
    }

    #[test]
    fn tableau_lisible_en_anglais() -> io::Result<()> {
        let mut out = Vec::new();
        write_table(&catalog(), None, &mut out)?;
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("ID "), "{text}");
        assert!(text.contains("LTS") && text.contains("2.0 GB"), "{text}");
        assert!(text.contains("official page"), "{text}");
        Ok(())
    }
}
