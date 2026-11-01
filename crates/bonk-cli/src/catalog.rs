//! `bonk catalog` : les images proposées, lues dans le catalogue signé, et `--image`, qui en écrit une.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use bonk_catalog::manifest::{
    self, Catalog, CatalogReport, Channel, FamilyEntry, Image, ImageSource, Loaded, TRUSTED_KEYS,
};
use bonk_hal::{Lang, format_size};
use clap::{Args, Subcommand, ValueEnum};
use serde::Deserialize;

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
    /// Keep one category (`Debian derivatives`); the families list gives them
    #[arg(long, value_name = "NAME")]
    category: Option<String>,
    /// Keep one architecture (`x86_64`, `aarch64`)
    #[arg(long, value_name = "ARCH")]
    arch: Option<String>,
    /// Keep one life cycle
    #[arg(long, value_enum)]
    channel: Option<ChannelArg>,
    /// Order of the list
    #[arg(long, value_enum, default_value_t = SortBy::System)]
    sort: SortBy,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub(crate) enum ChannelArg {
    Lts,
    Stable,
    EndOfLife,
}

/// Ordre de la liste. Le service reçoit les mêmes noms que la ligne de commande.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SortBy {
    /// By system, newest version first
    #[default]
    System,
    /// Most recently released first
    Released,
    /// Alphabetical
    Name,
    /// Smallest download first
    Size,
}

/// Ce que l'appelant garde de la liste, et dans quel ordre.
#[derive(Debug, Default, Clone)]
pub(crate) struct Query {
    pub(crate) family: Option<String>,
    pub(crate) category: Option<String>,
    pub(crate) arch: Option<String>,
    pub(crate) channel: Option<Channel>,
    pub(crate) sort: SortBy,
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
    if !image.usb_boot {
        return Err(JobError {
            code: Code::InvalidArgument,
            message: format!(
                "{} {} does not boot from a USB drive: {}",
                image.name,
                image.version,
                image.note.as_deref().unwrap_or("see its publisher")
            ),
        });
    }
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

/// Par défaut, les systèmes dans l'ordre du catalogue, la version la plus récente d'abord.
pub(crate) fn sorted<'a>(catalog: &'a Catalog, query: &Query) -> Vec<&'a Image> {
    let rank = |image: &Image| {
        catalog
            .families
            .iter()
            .position(|f| f.id == image.family)
            .unwrap_or(usize::MAX)
    };
    let category = |image: &Image| {
        catalog
            .families
            .iter()
            .find(|f| f.id == image.family)
            .and_then(|f| f.category.as_deref())
    };
    let size = |image: &Image| match image.source {
        ImageSource::Download { size_bytes, .. } => size_bytes,
        ImageSource::OfficialPage { .. } => u64::MAX,
    };
    let mut images: Vec<&Image> = catalog
        .images
        .iter()
        .filter(|image| {
            query
                .family
                .as_ref()
                .is_none_or(|wanted| &image.family == wanted)
        })
        .filter(|image| {
            query
                .category
                .as_deref()
                .is_none_or(|wanted| category(image) == Some(wanted))
        })
        .filter(|image| {
            query
                .arch
                .as_ref()
                .is_none_or(|wanted| &image.arch == wanted)
        })
        .filter(|image| query.channel.is_none_or(|wanted| image.channel == wanted))
        .collect();
    images.sort_by(|a, b| {
        let newest =
            manifest::version_numbers(&b.version).cmp(&manifest::version_numbers(&a.version));
        match query.sort {
            SortBy::System => rank(a).cmp(&rank(b)).then(newest),
            SortBy::Released => b.released.cmp(&a.released).then(newest),
            SortBy::Name => a.name.cmp(&b.name).then(newest),
            SortBy::Size => size(a).cmp(&size(b)),
        }
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

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(CatalogReport)).unwrap_or_default()
}

/// Le rapport JSON, partagé avec le service.
pub(crate) fn report_value(
    url: Option<&str>,
    cache_dir: Option<&Path>,
    query: &Query,
) -> Result<serde_json::Value, JobError> {
    let (loaded, dir) = load(url, cache_dir)?;
    let base = base_url(url, std::env::var("BONK_CATALOG_URL").ok());
    let fetch = |address: &str| bonk_catalog::fetch_bytes(address).map_err(|e| e.to_string());
    let catalog = &loaded.catalog;
    let report = CatalogReport {
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
                category: f.category.clone(),
                icon_path: manifest::icon_path(&base, &dir, f, &fetch),
            })
            .collect(),
        images: sorted(catalog, query).into_iter().cloned().collect(),
    };
    serde_json::to_value(report).map_err(refused)
}

fn write_table(catalog: &Catalog, query: &Query, out: &mut dyn Write) -> io::Result<()> {
    let rows: Vec<[String; 6]> = sorted(catalog, query)
        .into_iter()
        .map(|image| {
            let size = match &image.source {
                ImageSource::Download { size_bytes, .. } => format_size(*size_bytes, Lang::En),
                ImageSource::OfficialPage { .. } => "official page".to_owned(),
            };
            [
                image.id.clone(),
                format!("{} {}", image.name, image.version),
                image.arch.clone(),
                channel_label(image.channel).to_owned(),
                image.released.clone(),
                size,
            ]
        })
        .collect();
    let header = ["ID", "SYSTEM", "ARCH", "CHANNEL", "RELEASED", "SIZE"].map(str::to_owned);
    let mut widths = [0; 6];
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
    let query = Query {
        family: args.family.clone(),
        category: args.category.clone(),
        arch: args.arch.clone(),
        channel: args.channel.map(|channel| match channel {
            ChannelArg::Lts => Channel::Lts,
            ChannelArg::Stable => Channel::Stable,
            ChannelArg::EndOfLife => Channel::EndOfLife,
        }),
        sort: args.sort,
    };
    let written = if args.json {
        report_value(url, cache, &query).map(|value| {
            serde_json::to_writer_pretty(&mut *out, &value)
                .map_err(io::Error::from)
                .and_then(|()| writeln!(out))
        })
    } else {
        load(url, cache).map(|(loaded, _)| write_table(&loaded.catalog, &query, out))
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
            note: None,
            usb_boot: true,
            mirrors: Vec::new(),
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
            category: None,
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

    fn of_family(id: &str) -> Query {
        Query {
            family: Some(id.to_owned()),
            ..Query::default()
        }
    }

    fn ids<'a>(catalog: &'a Catalog, query: &Query) -> Vec<&'a str> {
        sorted(catalog, query)
            .iter()
            .map(|i| i.id.as_str())
            .collect()
    }

    #[test]
    fn filtres_par_categorie_architecture_et_cycle_de_vie() {
        let mut catalog = catalog();
        catalog.families[0].category = Some("Debian derivatives".to_owned());
        catalog.images[1].arch = "aarch64".to_owned();
        catalog.images[2].channel = Channel::EndOfLife;
        let query = |change: fn(&mut Query)| {
            let mut query = Query::default();
            change(&mut query);
            query
        };
        assert_eq!(
            ids(
                &catalog,
                &query(|q| q.category = Some("Debian derivatives".to_owned()))
            ),
            ["u22", "u26"]
        );
        assert_eq!(
            ids(&catalog, &query(|q| q.arch = Some("aarch64".to_owned()))),
            ["u22"]
        );
        assert_eq!(
            ids(&catalog, &query(|q| q.channel = Some(Channel::EndOfLife))),
            ["u26"]
        );
    }

    #[test]
    fn tris_par_date_nom_et_taille() {
        let catalog = catalog();
        let by = |sort| Query {
            sort,
            ..Query::default()
        };
        assert_eq!(ids(&catalog, &by(SortBy::Released)), ["u26", "u22", "w"]);
        assert_eq!(ids(&catalog, &by(SortBy::Name)), ["u22", "u26", "w"]);
        assert_eq!(
            ids(&catalog, &by(SortBy::Size)),
            ["u22", "u26", "w"],
            "sans taille : en dernier"
        );
    }

    #[test]
    fn une_image_qui_ne_demarre_pas_sur_cle_est_refusee_avant_le_telechargement() {
        let mut catalog = catalog();
        catalog.images[1].usb_boot = false;
        catalog.images[1].note = Some("CD only".to_owned());
        let message = resolve_image(&catalog, "u22")
            .err()
            .map(|e| e.message)
            .unwrap_or_default();
        assert!(message.contains("CD only"), "{message}");
    }

    #[test]
    fn systemes_dans_l_ordre_et_versions_recentes_d_abord() {
        let catalog = catalog();
        let ids: Vec<&str> = sorted(&catalog, &Query::default())
            .iter()
            .map(|i| i.id.as_str())
            .collect();
        assert_eq!(ids, ["u22", "u26", "w"]);
        assert_eq!(sorted(&catalog, &of_family("windows")).len(), 1);
        // Une version plus ancienne sortie plus tard reste après la plus récente.
        let mut tardive = catalog.clone();
        tardive.images[1].version = "24.04.5".to_owned();
        tardive.images[1].released = "2027-01-01".to_owned();
        tardive.images[2].version = "26.04.1".to_owned();
        let ids: Vec<&str> = sorted(&tardive, &of_family("ubuntu"))
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
        write_table(&catalog(), &Query::default(), &mut out)?;
        let text = String::from_utf8_lossy(&out);
        assert!(text.starts_with("ID "), "{text}");
        assert!(text.contains("LTS") && text.contains("2.0 GB"), "{text}");
        assert!(text.contains("official page"), "{text}");
        Ok(())
    }
}
