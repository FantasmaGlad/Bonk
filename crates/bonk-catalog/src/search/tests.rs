use serde::Deserialize;

use super::*;

#[derive(Deserialize)]
struct Table {
    families: Vec<CaseFamily>,
    images: Vec<CaseImage>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct CaseFamily {
    id: String,
    name: String,
    uses: Vec<String>,
    category: Option<String>,
}

#[derive(Deserialize)]
struct CaseImage {
    id: String,
    family: String,
    name: String,
    edition: Option<String>,
    desktop: Option<String>,
    media: Option<String>,
    usb_boot: bool,
    arch: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct CaseQuery {
    text: String,
    uses: Option<String>,
    base: Option<String>,
    desktop: Option<String>,
    media: Option<String>,
    arch: Option<String>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    query: CaseQuery,
    images: Vec<String>,
}

fn kept(table: &Table, query: &Query) -> Vec<String> {
    table
        .images
        .iter()
        .filter(|image| {
            let family = table.families.iter().find(|f| f.id == image.family);
            family.is_some_and(|family| {
                let family = FamilyFacts {
                    id: &family.id,
                    name: &family.name,
                    uses: &family.uses,
                    base: family.category.as_deref(),
                };
                let facts = ImageFacts {
                    name: &image.name,
                    edition: image.edition.as_deref(),
                    desktop: image.desktop.as_deref(),
                    media: media_of(image.media.as_deref(), image.usb_boot),
                    arch: &image.arch,
                };
                query.keeps_family(family) && query.keeps_image(family, facts)
            })
        })
        .map(|image| image.id.clone())
        .collect()
}

#[test]
fn la_table_de_cas_partagee_avec_le_site() -> Result<(), serde_json::Error> {
    let table: Table = serde_json::from_str(include_str!("cases.json"))?;
    for case in &table.cases {
        let query = Query {
            text: case.query.text.clone(),
            uses: case.query.uses.clone(),
            base: case.query.base.clone(),
            desktop: case.query.desktop.clone(),
            media: case.query.media.clone(),
            arch: case.query.arch.clone(),
        };
        assert_eq!(kept(&table, &query), case.images, "{}", case.name);
    }
    Ok(())
}

#[test]
fn les_accents_decomposables_tombent_les_autres_restent() {
    assert_eq!(fold("Élan ÇA Ŝ"), "elan ca s");
    assert_eq!(fold("e\u{301}"), "e");
    assert_eq!(fold("Øß"), "øß");
}
