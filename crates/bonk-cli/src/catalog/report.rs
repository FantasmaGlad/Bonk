use std::io::{self, Write};
use std::path::Path;

use bonk_catalog::manifest::{Catalog, CatalogReport, Channel, FamilyEntry, ImageSource};
use bonk_hal::{Lang, format_size};
use bonk_ops::job::{self, JobError};

use super::{Query, base_url, load, refused, sorted};

fn channel_label(channel: Channel) -> &'static str {
    match channel {
        Channel::Lts => "LTS",
        Channel::Stable => "stable",
        Channel::EndOfLife => "END OF LIFE",
        Channel::Archive => "ARCHIVE",
    }
}

pub(crate) fn schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(CatalogReport)).unwrap_or_default()
}

/// Shared with the service.
/// Logos downloaded at a time on a cold cache: enough to hide the latency, few enough for the server.
const ICON_WORKERS: usize = 8;

pub(crate) fn report_value(
    url: Option<&str>,
    cache_dir: Option<&Path>,
    query: &Query,
) -> Result<serde_json::Value, JobError> {
    let (loaded, dir) = load(url, cache_dir)?;
    let base = base_url(url, std::env::var("BONK_CATALOG_URL").ok());
    let fetch = |address: &str| bonk_catalog::fetch_bytes(address).map_err(|e| e.to_string());
    let catalog = &loaded.catalog;
    let icons = bonk_catalog::icons::paths(&base, &dir, &catalog.families, ICON_WORKERS, &fetch);
    let report = CatalogReport {
        schema_version: job::SCHEMA_VERSION,
        sequence: catalog.sequence,
        expires_unix: catalog.expires_unix,
        saved_copy: loaded.saved_copy,
        families: catalog
            .families
            .iter()
            .zip(icons)
            .map(|(f, icon_path)| FamilyEntry {
                id: f.id.clone(),
                name: f.name.clone(),
                category: f.category.clone(),
                uses: f.uses.clone(),
                not_affiliated_with: f.not_affiliated_with.clone(),
                icon_path,
            })
            .collect(),
        images: sorted(catalog, query).into_iter().cloned().collect(),
        taxonomy: catalog.taxonomy.clone(),
    };
    serde_json::to_value(report).map_err(refused)
}

pub(super) fn write_table(catalog: &Catalog, query: &Query, out: &mut dyn Write) -> io::Result<()> {
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
