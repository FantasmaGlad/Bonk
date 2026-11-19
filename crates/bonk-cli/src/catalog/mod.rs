use std::io::{self, Write};

use bonk_catalog::manifest::Channel;
use bonk_catalog::search;
use bonk_ops::job;

use crate::errors::{Code, report};

mod access;
mod args;
pub(crate) mod download;
mod query;
mod report;

pub(crate) use access::*;
pub(crate) use args::*;
pub(crate) use query::*;
pub(crate) use report::*;

pub(crate) fn run(
    action: &CatalogAction,
    deps: &job::Deps<'_>,
    quiet: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    match action {
        CatalogAction::List(args) => run_list(args, out, err),
        CatalogAction::Download(args) => download::run(args, deps, quiet, out, err),
    }
}

fn run_list(args: &ListArgs, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    let url = args.catalog_url.as_deref();
    let cache = args.cache_dir.as_deref();
    let query = Query {
        family: args.family.clone(),
        filter: search::Query {
            text: args.search.clone().unwrap_or_default(),
            uses: args.uses.clone(),
            base: args.category.clone(),
            desktop: args.desktop.clone(),
            media: args.media.clone(),
            arch: args.arch.clone(),
        },
        channel: args.channel.map(|channel| match channel {
            ChannelArg::Lts => Channel::Lts,
            ChannelArg::Stable => Channel::Stable,
            ChannelArg::EndOfLife => Channel::EndOfLife,
            ChannelArg::Archive => Channel::Archive,
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
    use bonk_catalog::manifest::{self, Catalog, Family, Image, ImageSource};
    use bonk_ops::job::JobError;

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
            desktop: None,
            kind: None,
            media: None,
            ram_mb: None,
            ram_source: None,
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
            parent: None,
            homepage: None,
            icon_source: None,
            uses: Vec::new(),
            ram_mb: None,
            ram_source: None,
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
            grub: None,
            taxonomy: None,
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
                &query(|q| q.filter.base = Some("Debian derivatives".to_owned()))
            ),
            ["u22", "u26"]
        );
        assert_eq!(
            ids(
                &catalog,
                &query(|q| q.filter.arch = Some("aarch64".to_owned()))
            ),
            ["u22"]
        );
        assert_eq!(
            ids(&catalog, &query(|q| q.channel = Some(Channel::EndOfLife))),
            ["u26"]
        );
    }

    #[test]
    fn recherche_par_mots_usage_bureau_et_support_comme_la_fenetre() {
        let mut catalog = catalog();
        catalog.families[0].uses = vec!["office".to_owned()];
        catalog.images[1].name = "Kubuntu".to_owned();
        catalog.images[1].desktop = Some("kde".to_owned());
        catalog.images[2].usb_boot = false;
        let query = |filter: search::Query| Query {
            filter,
            ..Query::default()
        };
        let text = |text: &str| search::Query {
            text: text.to_owned(),
            ..search::Query::default()
        };
        assert_eq!(ids(&catalog, &query(text(" KUBU "))), ["u22"]);
        assert_eq!(ids(&catalog, &query(text("ubuntu"))), ["u22", "u26"]);
        let office = search::Query {
            uses: Some("office".to_owned()),
            ..search::Query::default()
        };
        assert_eq!(ids(&catalog, &query(office)), ["u22", "u26"]);
        let kde = search::Query {
            desktop: Some("kde".to_owned()),
            ..search::Query::default()
        };
        assert_eq!(ids(&catalog, &query(kde)), ["u22"]);
        let optical = search::Query {
            media: Some("optical".to_owned()),
            ..search::Query::default()
        };
        assert_eq!(ids(&catalog, &query(optical)), ["u26"]);
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
        // An older version released later stays after the newest.
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
        assert_eq!(base_url(None, None), manifest::DEFAULT_BASE);
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
