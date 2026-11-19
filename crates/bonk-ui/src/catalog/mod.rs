use std::path::PathBuf;

use bonk_catalog::manifest::{CatalogReport, Channel, Image, ImageSource};
use bonk_catalog::search::{self, FamilyFacts, ImageFacts};
use bonk_hal::Lang;

mod menu;
pub use menu::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// Catalog identifier: what `--image` and a `bonk://install/` link name.
    pub id: String,
    /// May differ from the system: Kubuntu is filed under Ubuntu.
    pub name: String,
    pub edition: Option<String>,
    pub label: String,
    pub end_of_life: bool,
    /// What to know before choosing it (evaluation, CD only).
    pub note: Option<String>,
    pub source: ChoiceSource,
    /// Catalog desktop id: `gnome`, `none`, `choice`.
    pub desktop: Option<String>,
    /// Catalog medium id: `usb`, `sd`, `optical`.
    pub media: String,
    /// Processor the image boots on: `x86_64`, `aarch64`, `riscv64`.
    pub arch: String,
}

impl Choice {
    #[must_use]
    pub fn facts(&self) -> ImageFacts<'_> {
        ImageFacts {
            name: &self.name,
            edition: self.edition.as_deref(),
            desktop: self.desktop.as_deref(),
            media: &self.media,
            arch: &self.arch,
        }
    }

    /// The label comes from the catalog; what Bonk adds follows the window language.
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
        /// Set only when the address does not end with the file's name.
        file_name: Option<String>,
    },
    /// No stable link at the publisher: the address of its page, to open oneself.
    OfficialPage(String),
    /// Does not boot from a key: shown, not writable.
    NotForUsb,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    /// Catalog family identifier: `ubuntu`.
    pub id: String,
    pub name: String,
    /// Usage categories of the catalog: `office`, `server`.
    pub uses: Vec<String>,
    /// Technical base: `debian`.
    pub base: Option<String>,
    pub icon: Option<PathBuf>,
    pub choices: Vec<Choice>,
}

impl System {
    #[must_use]
    pub fn facts(&self) -> FamilyFacts<'_> {
        FamilyFacts {
            id: &self.id,
            name: &self.name,
            uses: &self.uses,
            base: self.base.as_deref(),
        }
    }
}

/// The catalog as the window shows it: its systems and the labels of its menus.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shelf {
    pub systems: Vec<System>,
    pub terms: crate::terms::Terms,
    /// The last verified copy: the network did not answer.
    pub saved_copy: bool,
}

/// For an image published only on its editor's page: Bonk cannot download it itself.
#[must_use]
pub fn official_page_note(page: &str, lang: Lang) -> String {
    match lang {
        Lang::En => format!("download from {page}, then Other image"),
        Lang::Fr => format!("à télécharger sur {page}, puis Autre image"),
    }
}

#[derive(Debug, Default)]
pub enum CatalogState {
    #[default]
    NotLoaded,
    Loading,
    Ready(Vec<System>),
    Failed(String),
}

impl CatalogState {
    /// The name the catalog gives the file at this address, when the address does not end with it.
    #[must_use]
    pub fn file_name_of(&self, address: &str) -> Option<&str> {
        let Self::Ready(systems) = self else {
            return None;
        };
        match &find_url(systems, address)?.1.source {
            ChoiceSource::Download { file_name, .. } => file_name.as_deref(),
            _ => None,
        }
    }
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
            file_name: image.file_name.clone(),
        },
        ImageSource::OfficialPage { official_page } => {
            ChoiceSource::OfficialPage(official_page.clone())
        }
    };
    Choice {
        id: image.id.clone(),
        name: image.name.clone(),
        edition: image.edition.clone(),
        label: label_of(image),
        // An archived version is no longer maintained either: the interface says so the same way.
        end_of_life: matches!(image.channel, Channel::EndOfLife | Channel::Archive),
        note: image.note.clone(),
        source,
        desktop: image.desktop.clone(),
        media: search::media_of(image.media.as_deref(), image.usb_boot).to_owned(),
        arch: image.arch.clone(),
    }
}

/// The report already sorts each system's images newest first.
#[must_use]
pub fn systems(report: &CatalogReport) -> Vec<System> {
    report
        .families
        .iter()
        .filter_map(|family| {
            let mut choices: Vec<Choice> = report
                .images
                .iter()
                .filter(|image| image.family == family.id)
                .map(choice)
                .collect();
            // What Bonk can write comes first: a CD-only file is listed, never offered ahead.
            choices.sort_by_key(|c| matches!(c.source, ChoiceSource::NotForUsb));
            (!choices.is_empty()).then(|| System {
                id: family.id.clone(),
                name: family.name.clone(),
                uses: family.uses.clone(),
                base: family.category.clone(),
                icon: family.icon_path.clone(),
                choices,
            })
        })
        .collect()
}

#[must_use]
pub fn shelf(report: &CatalogReport) -> Shelf {
    Shelf {
        systems: systems(report),
        terms: crate::terms::Terms::from_catalog(report.taxonomy.as_ref()),
        saved_copy: report.saved_copy,
    }
}

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

/// The catalog image published under this hash: what a kept download was checked against.
pub fn find_sha256<'a>(systems: &'a [System], sha256: &str) -> Option<&'a Choice> {
    systems.iter().flat_map(|system| &system.choices).find(
        |choice| matches!(&choice.source, ChoiceSource::Download { sha256: s, .. } if s == sha256),
    )
}

pub fn find_id<'a>(systems: &'a [System], id: &str) -> Option<(&'a System, &'a Choice)> {
    systems.iter().find_map(|system| {
        system
            .choices
            .iter()
            .find(|choice| choice.id == id)
            .map(|choice| (system, choice))
    })
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

    /// Adds to each image what the test does not look at.
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
                {"id": "ubuntu", "name": "Ubuntu", "uses": ["desktop", "server"], "icon_path": "/c/u.png"},
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
    fn listes_du_menu_triees_sans_doublons() {
        let system = |name: &str, used: &str, base: &str| System {
            id: String::new(),
            name: name.to_owned(),
            uses: vec![used.to_owned()],
            base: Some(base.to_owned()),
            icon: None,
            choices: Vec::new(),
        };
        let systems = [
            system("Ubuntu", "server", "ubuntu"),
            system("Windows", "windows", "windows"),
            system("Debian", "office", "debian"),
            system("Kali", "security", "debian"),
        ];
        let terms = crate::terms::Terms::default();
        let tout = MenuFilter::default().lists(&systems, &terms);
        assert_eq!(tout.categories, ["office", "security", "server", "windows"]);
        assert_eq!(tout.bases, ["debian", "ubuntu", "windows"]);
        assert_eq!(tout.systems, ["Debian", "Kali", "Ubuntu", "Windows"]);
        let debian = MenuFilter {
            base: Some("debian".to_owned()),
            ..MenuFilter::default()
        };
        assert_eq!(debian.lists(&systems, &terms).systems, ["Debian", "Kali"]);
        let securite = MenuFilter {
            category: Some("security".to_owned()),
            system: Some("Kali".to_owned()),
            ..debian
        };
        assert_eq!(securite.lists(&systems, &terms).systems, ["Kali"]);
        assert_eq!(securite.apply(&systems).len(), 1);
    }

    #[test]
    fn la_taxonomie_du_rapport_donne_les_libelles() {
        let mut report = report();
        report["taxonomy"] =
            json!({"uses": [{"id": "server", "en": "Servers", "fr": "Serveurs pro"}]});
        let shelf = serde_json::from_value(report)
            .map(|report| shelf(&report))
            .unwrap_or_default();
        assert_eq!(shelf.systems.len(), 2);
        assert_eq!(shelf.terms.uses().label("server", Lang::Fr), "Serveurs pro");
        assert_eq!(shelf.terms.bases().label("ubuntu", Lang::Fr), "Ubuntu");
    }

    #[test]
    fn systemes_avec_logo_et_versions_dans_l_ordre_recu() {
        let systems = parse(&report());
        assert_eq!(systems.len(), 2, "un système sans image n'est pas montré");
        assert_eq!(systems[0].icon, Some(PathBuf::from("/c/u.png")));
        assert_eq!(systems[0].choices[0].label, "26.04.1 desktop LTS");
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

        assert_eq!(
            MenuFilter::default().writable(&systems).len(),
            2,
            "une image de CD ne s'écrit pas"
        );
        assert!(matches!(
            find_url(&systems, "https://u/20.iso"),
            Some((system, choice)) if system.name == "Ubuntu" && choice.end_of_life
        ));
        assert!(find_url(&systems, "https://ailleurs/").is_none());
    }

    #[test]
    fn une_categorie_ou_un_seul_systeme() {
        let systems = parse(&report());
        assert!(systems[1].uses.is_empty());
        let server = MenuFilter {
            category: Some("server".to_owned()),
            ..MenuFilter::default()
        };
        assert_eq!(server.apply(&systems).len(), 1);
        let windows = MenuFilter {
            system: Some("Windows".to_owned()),
            ..MenuFilter::default()
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

    #[test]
    fn la_recherche_trouve_une_image_par_son_nom_ou_par_son_systeme() {
        let systems = parse(&report());
        let filter = |search: &str| MenuFilter {
            search: search.to_owned(),
            ..MenuFilter::default()
        };
        let names = |f: &MenuFilter| -> Vec<String> {
            f.apply(&systems)
                .into_iter()
                .flat_map(|s| f.choices(s))
                .map(|c| c.name.clone())
                .collect()
        };
        assert_eq!(names(&filter("KUBUNTU")), ["Kubuntu"]);
        assert_eq!(names(&filter("ubuntu")).len(), 3);
        assert_eq!(names(&filter("WINDOWS")), ["windows"]);
        assert!(filter("rien").apply(&systems).is_empty());
    }

    /// The rows shown and the keyboard index count the same images: one filter decides both.
    #[test]
    fn le_processeur_retient_ses_images_et_l_index_clavier_suit() {
        let systems = parse(&report());
        let arm = MenuFilter {
            arch: Some("aarch64".to_owned()),
            ..MenuFilter::default()
        };
        let shown: Vec<&str> = arm
            .apply(&systems)
            .into_iter()
            .flat_map(|s| arm.choices(s))
            .map(|c| c.arch.as_str())
            .collect();
        assert_eq!(shown, ["aarch64"]);
        assert!(arm.writable(&systems).is_empty(), "l'image ARM est un CD");
        let x86 = MenuFilter {
            arch: Some("x86_64".to_owned()),
            ..MenuFilter::default()
        };
        assert_eq!(x86.writable(&systems).len(), 2);
        assert_eq!(
            MenuFilter::default()
                .lists(&systems, &crate::terms::Terms::default())
                .arches,
            ["x86_64", "aarch64"]
        );
        assert_eq!(arch_label("aarch64", Lang::Fr), "aarch64 (ARM, 64 bits)");
    }

    #[test]
    fn le_nom_donne_par_le_catalogue_suit_son_adresse_dans_la_commande() {
        let partage = "https://share.example/files/686/download";
        let mut report = report();
        report["images"][0]["url"] = json!(partage);
        report["images"][0]["file_name"] = json!("GLF-OS-STABLE.iso");
        let catalog = CatalogState::Ready(parse(&report));
        assert_eq!(catalog.file_name_of(partage), Some("GLF-OS-STABLE.iso"));
        assert_eq!(catalog.file_name_of("https://u/20.iso"), None);
        assert_eq!(catalog.file_name_of("https://ailleurs/download"), None);
        assert_eq!(CatalogState::Loading.file_name_of(partage), None);
        let flags = crate::command::WriteView {
            file_name: catalog.file_name_of(partage),
            ..crate::command::WriteView::image(partage)
        }
        .flags();
        assert!(flags.ends_with(&["--file-name".to_owned(), "GLF-OS-STABLE.iso".to_owned()]));
    }
}
