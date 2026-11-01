//! Le menu des images : ce qu'on en retient pour l'afficher, quelle que soit l'interface.

use std::path::PathBuf;

use bonk_catalog::manifest::{CatalogReport, Channel, Image, ImageSource};
use bonk_hal::Lang;

/// Une version proposée d'un système.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Peut différer du système : Kubuntu est rangé sous Ubuntu.
    pub name: String,
    pub label: String,
    pub end_of_life: bool,
    /// Ce qu'il faut savoir avant de la choisir (évaluation, CD seulement).
    pub note: Option<String>,
    pub source: ChoiceSource,
}

impl Choice {
    /// Le libellé vient du catalogue ; ce qu'y ajoute Bonk suit la langue de la fenêtre.
    pub fn title(&self, lang: Lang) -> String {
        if self.end_of_life {
            format!(
                "{} {}",
                self.label,
                lang.pick("(end of life)", "(fin de vie)")
            )
        } else {
            self.label.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceSource {
    Download {
        url: String,
        sha256: String,
    },
    /// Pas de lien stable chez l'éditeur : l'adresse de sa page, à ouvrir soi-même.
    OfficialPage(String),
    /// Ne démarre pas d'une clé : montrée sans pouvoir être écrite.
    NotForUsb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    pub name: String,
    pub category: String,
    pub icon: Option<PathBuf>,
    pub choices: Vec<Choice>,
}

/// Ce que le menu garde : une catégorie, ou un seul système et ses versions. Vide : tout.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MenuFilter {
    pub category: Option<String>,
    pub system: Option<String>,
}

impl MenuFilter {
    pub fn apply<'a>(&self, systems: &'a [System]) -> Vec<&'a System> {
        systems
            .iter()
            .filter(|s| self.category.as_ref().is_none_or(|c| &s.category == c))
            .filter(|s| self.system.as_ref().is_none_or(|n| &s.name == n))
            .collect()
    }
}

/// Sans catégorie dans le catalogue : un seul groupe, celui de tout le monde.
const NO_CATEGORY: &str = "Other systems";

#[derive(Debug, Default)]
pub enum CatalogState {
    #[default]
    NotLoaded,
    Loading,
    Ready(Vec<System>),
    Failed(String),
}

fn label_of(image: &Image) -> String {
    let mut label = image.version.clone();
    if let Some(edition) = &image.edition {
        label.push(' ');
        label.push_str(edition);
    }
    if image.arch != "x86_64" {
        label.push(' ');
        label.push_str(&image.arch);
    }
    if image.channel == Channel::Lts {
        label.push_str(" LTS");
    }
    label
}

fn choice(image: &Image) -> Choice {
    let source = match &image.source {
        _ if !image.usb_boot => ChoiceSource::NotForUsb,
        ImageSource::Download { url, sha256, .. } => ChoiceSource::Download {
            url: url.clone(),
            sha256: sha256.clone(),
        },
        ImageSource::OfficialPage { official_page } => {
            ChoiceSource::OfficialPage(official_page.clone())
        }
    };
    Choice {
        name: image.name.clone(),
        label: label_of(image),
        end_of_life: image.channel == Channel::EndOfLife,
        note: image.note.clone(),
        source,
    }
}

/// Les systèmes du rapport : ses images y sont déjà rangées, de la version la plus récente à la plus ancienne.
#[must_use]
pub fn systems(report: &CatalogReport) -> Vec<System> {
    report
        .families
        .iter()
        .filter_map(|family| {
            let choices: Vec<Choice> = report
                .images
                .iter()
                .filter(|image| image.family == family.id)
                .map(choice)
                .collect();
            (!choices.is_empty()).then(|| System {
                name: family.name.clone(),
                category: family
                    .category
                    .clone()
                    .unwrap_or_else(|| NO_CATEGORY.to_owned()),
                icon: family.icon_path.clone(),
                choices,
            })
        })
        .collect()
}

/// Les versions qu'un clic peut écrire, dans l'ordre du menu : l'indice du clavier s'y rapporte.
pub fn writable<'a>(systems: &[&'a System]) -> Vec<&'a Choice> {
    systems
        .iter()
        .flat_map(|system| &system.choices)
        .filter(|choice| matches!(choice.source, ChoiceSource::Download { .. }))
        .collect()
}

/// Le système et la version dont `url` est l'adresse.
pub fn find_url<'a>(systems: &'a [System], url: &str) -> Option<(&'a System, &'a Choice)> {
    systems.iter().find_map(|system| {
        system
            .choices
            .iter()
            .find(|choice| {
                matches!(&choice.source, ChoiceSource::Download { url: u, .. } if u == url)
            })
            .map(|choice| (system, choice))
    })
}

/// Libellé du choix « toutes les catégories » du filtre.
#[must_use]
pub fn all_categories(lang: Lang) -> &'static str {
    lang.pick("All categories", "Toutes les catégories")
}

/// Libellé du choix « tous les systèmes » du filtre.
#[must_use]
pub fn all_systems(lang: Lang) -> &'static str {
    lang.pick("All systems", "Tous les systèmes")
}
#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn parse(report: &Value) -> Vec<System> {
        serde_json::from_value(report.clone())
            .map(|report| systems(&report))
            .unwrap_or_default()
    }

    /// Ajoute à chaque image ce que le test ne regarde pas.
    fn complete(mut report: Value) -> Value {
        report["schema_version"] = json!(1);
        report["sequence"] = json!(1);
        report["expires_unix"] = json!(1);
        report["saved_copy"] = json!(false);
        for (index, image) in report["images"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .enumerate()
        {
            for (key, value) in [
                ("id", json!(format!("i{index}"))),
                ("name", image["family"].clone()),
                ("released", json!("2026-01-01")),
                ("arch", json!("x86_64")),
                ("homepage", json!("https://h/")),
                ("size_bytes", json!(1)),
            ] {
                if image.get(key).is_none() {
                    image[key] = value;
                }
            }
        }
        report
    }

    fn report() -> Value {
        complete(json!({
            "families": [
                {"id": "ubuntu", "name": "Ubuntu", "category": "Debian derivatives", "icon_path": "/c/u.png"},
                {"id": "vide", "name": "Vide"},
                {"id": "windows", "name": "Windows"}
            ],
            "images": [
                {"family": "ubuntu", "version": "26.04.1", "edition": "desktop", "channel": "lts",
                 "url": "https://u/26.iso", "sha256": "aa"},
                {"family": "ubuntu", "version": "20.04.5", "edition": "desktop", "channel": "end_of_life",
                 "url": "https://u/20.iso", "sha256": "bb"},
                {"family": "windows", "version": "11", "channel": "stable", "official_page": "https://p/"},
                {"family": "ubuntu", "name": "Kubuntu", "version": "1", "channel": "stable", "arch": "aarch64", "usb_boot": false,
                 "note": "CD", "url": "https://u/cd.iso", "sha256": "cc"}
            ]
        }))
    }

    #[test]
    fn systemes_avec_logo_et_versions_dans_l_ordre_recu() {
        let systems = parse(&report());
        assert_eq!(systems.len(), 2, "un système sans image n'est pas montré");
        assert_eq!(systems[0].icon, Some(PathBuf::from("/c/u.png")));
        assert_eq!(systems[0].choices[0].label, "26.04.1 desktop LTS");
        assert_eq!(systems[0].choices[0].name, "Ubuntu",);
        assert_eq!(
            systems[0].choices[1].title(Lang::Fr),
            "20.04.5 desktop (fin de vie)"
        );
        assert_eq!(
            systems[0].choices[1].title(Lang::En),
            "20.04.5 desktop (end of life)"
        );
        assert!(systems[0].choices[1].end_of_life);
        assert_eq!(
            systems[1].choices[0].source,
            ChoiceSource::OfficialPage("https://p/".to_owned())
        );
    }

    #[test]
    fn le_menu_clavier_ne_compte_que_ce_qui_s_ecrit_et_retrouve_un_choix() {
        let systems = parse(&report());
        let all = MenuFilter::default().apply(&systems);
        assert_eq!(writable(&all).len(), 2, "une image de CD ne s'écrit pas");
        assert!(matches!(
            find_url(&systems, "https://u/20.iso"),
            Some((system, choice)) if system.name == "Ubuntu" && choice.end_of_life
        ));
        assert!(find_url(&systems, "https://ailleurs/").is_none());
    }

    #[test]
    fn une_categorie_ou_un_seul_systeme() {
        let systems = parse(&report());
        assert_eq!(systems[1].category, NO_CATEGORY);
        let debian = MenuFilter {
            category: Some("Debian derivatives".to_owned()),
            system: None,
        };
        assert_eq!(debian.apply(&systems).len(), 1);
        let windows = MenuFilter {
            category: None,
            system: Some("Windows".to_owned()),
        };
        assert_eq!(windows.apply(&systems)[0].name, "Windows");
        let cd = &systems[0].choices[2];
        assert_eq!(
            (cd.label.as_str(), cd.source.clone()),
            ("1 aarch64", ChoiceSource::NotForUsb)
        );
        assert_eq!(cd.note.as_deref(), Some("CD"));
        assert_eq!(cd.name, "Kubuntu", "une variante garde son propre nom");
    }

    #[test]
    fn rapport_illisible_rien_a_proposer() {
        assert!(parse(&json!({"autre": 1})).is_empty());
    }
}
