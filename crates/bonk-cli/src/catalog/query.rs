use bonk_catalog::manifest::Channel;
use bonk_catalog::manifest::{self, Catalog, Image, ImageSource};
use bonk_catalog::search::{self, FamilyFacts, ImageFacts};

use super::SortBy;

#[derive(Debug, Default, Clone)]
pub(crate) struct Query {
    pub(crate) family: Option<String>,
    pub(crate) filter: search::Query,
    pub(crate) channel: Option<Channel>,
    pub(crate) sort: SortBy,
}

fn selected(catalog: &Catalog, query: &Query, image: &Image) -> bool {
    let family = catalog.families.iter().find(|f| f.id == image.family);
    family.is_some_and(|family| {
        let facts = FamilyFacts::of(family);
        query
            .family
            .as_ref()
            .is_none_or(|wanted| &family.id == wanted)
            && query.channel.is_none_or(|wanted| image.channel == wanted)
            && query.filter.keeps_family(facts)
            && query.filter.keeps_image(facts, ImageFacts::of(image))
    })
}

pub(crate) fn sorted<'a>(catalog: &'a Catalog, query: &Query) -> Vec<&'a Image> {
    let rank = |image: &Image| {
        catalog
            .families
            .iter()
            .position(|f| f.id == image.family)
            .unwrap_or(usize::MAX)
    };
    let size = |image: &Image| match image.source {
        ImageSource::Download { size_bytes, .. } => size_bytes,
        ImageSource::OfficialPage { .. } => u64::MAX,
    };
    let mut images: Vec<&Image> = catalog
        .images
        .iter()
        .filter(|image| selected(catalog, query, image))
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
