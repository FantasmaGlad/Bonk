//! Le menu des images : la fenêtre lit `bonk catalog list --json`, comme elle écrit par `bonk write`. Signature
//! et expiration restent donc contrôlées au même endroit ; un choix donne l'adresse et l'empreinte officielles,
//! sans rien à régler.

use std::path::PathBuf;

use bonk_hal::Lang;
use iced::widget::{button, column, container, image, pick_list, row, scrollable, text};
use iced::{Element, Length};
use serde_json::Value;

use crate::theme;

use super::Message;

/// Une version proposée d'un système.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    /// Peut différer du système : Kubuntu est rangé sous Ubuntu.
    pub(crate) name: String,
    pub(crate) label: String,
    pub(crate) end_of_life: bool,
    /// Ce qu'il faut savoir avant de la choisir (évaluation, CD seulement).
    pub(crate) note: Option<String>,
    pub(crate) source: ChoiceSource,
}

impl Choice {
    /// Le libellé vient du catalogue ; ce qu'y ajoute Bonk suit la langue de la fenêtre.
    pub(crate) fn title(&self, lang: Lang) -> String {
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
pub(crate) enum ChoiceSource {
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
pub(crate) struct System {
    pub(crate) name: String,
    pub(crate) category: String,
    pub(crate) icon: Option<PathBuf>,
    pub(crate) choices: Vec<Choice>,
}

/// Ce que le menu garde : une catégorie, ou un seul système et ses versions. Vide : tout.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MenuFilter {
    pub(crate) category: Option<String>,
    pub(crate) system: Option<String>,
}

impl MenuFilter {
    pub(crate) fn apply<'a>(&self, systems: &'a [System]) -> Vec<&'a System> {
        systems
            .iter()
            .filter(|s| self.category.as_ref().is_none_or(|c| &s.category == c))
            .filter(|s| self.system.as_ref().is_none_or(|n| &s.name == n))
            .collect()
    }
}

fn all_categories(lang: Lang) -> &'static str {
    lang.pick("All categories", "Toutes les catégories")
}

fn all_systems(lang: Lang) -> &'static str {
    lang.pick("All systems", "Tous les systèmes")
}

/// Sans catégorie dans le catalogue : un seul groupe, celui de tout le monde.
const NO_CATEGORY: &str = "Other systems";

#[derive(Debug, Default)]
pub(crate) enum CatalogState {
    #[default]
    NotLoaded,
    Loading,
    Ready(Vec<System>),
    Failed(String),
}

fn label_of(image: &Value) -> Option<String> {
    let version = image["version"].as_str()?;
    let mut label = version.to_owned();
    if let Some(edition) = image["edition"].as_str() {
        label.push(' ');
        label.push_str(edition);
    }
    if let Some(arch) = image["arch"].as_str().filter(|arch| *arch != "x86_64") {
        label.push(' ');
        label.push_str(arch);
    }
    if image["channel"] == "lts" {
        label.push_str(" LTS");
    }
    Some(label)
}

/// Lit le rapport : les images y sont déjà rangées, de la version la plus récente à la plus ancienne.
pub(crate) fn parse(report: &Value) -> Vec<System> {
    let images = report["images"].as_array().map_or(&[][..], Vec::as_slice);
    report["families"]
        .as_array()
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .filter_map(|family| {
            let id = family["id"].as_str()?;
            let choices: Vec<Choice> = images
                .iter()
                .filter(|image| image["family"] == id)
                .filter_map(|image| {
                    let source = match (image["url"].as_str(), image["sha256"].as_str()) {
                        _ if image["usb_boot"] == false => ChoiceSource::NotForUsb,
                        (Some(url), Some(sha256)) => ChoiceSource::Download {
                            url: url.to_owned(),
                            sha256: sha256.to_owned(),
                        },
                        _ => {
                            ChoiceSource::OfficialPage(image["official_page"].as_str()?.to_owned())
                        }
                    };
                    Some(Choice {
                        name: image["name"]
                            .as_str()
                            .or(family["name"].as_str())
                            .unwrap_or(id)
                            .to_owned(),
                        label: label_of(image)?,
                        end_of_life: image["channel"] == "end_of_life",
                        note: image["note"].as_str().map(str::to_owned),
                        source,
                    })
                })
                .collect();
            (!choices.is_empty()).then(|| System {
                name: family["name"].as_str().unwrap_or(id).to_owned(),
                category: family["category"]
                    .as_str()
                    .unwrap_or(NO_CATEGORY)
                    .to_owned(),
                icon: family["icon_path"].as_str().map(PathBuf::from),
                choices,
            })
        })
        .collect()
}

/// Lit le catalogue dans un processus séparé. La dernière ligne d'erreur dit pourquoi il a échoué.
pub(crate) fn read(program: PathBuf) -> Result<Vec<System>, String> {
    let mut command = std::process::Command::new(&program);
    bonk_hal::console::hide_child_console(&mut command);
    let output = command
        .args(["catalog", "list", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("{}: cannot start: {e}", program.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr
            .lines()
            .rev()
            .find(|l| l.starts_with("bonk: error: "));
        return Err(line.map_or_else(
            || output.status.to_string(),
            |l| l["bonk: error: ".len()..].to_owned(),
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map(|report: Value| parse(&report))
        .map_err(|e| e.to_string())
}

/// Identifiant de la liste du menu, pour la suivre au clavier.
pub(super) const MENU: &str = "menu-images";

fn logo(icon: Option<&PathBuf>) -> Element<'_, Message> {
    match icon {
        Some(path) => image(image::Handle::from_path(path))
            .width(Length::Fixed(24.0))
            .height(Length::Fixed(24.0))
            .into(),
        None => container(text("")).width(Length::Fixed(24.0)).into(),
    }
}

/// Les versions qu'un clic peut écrire, dans l'ordre du menu : l'indice du clavier s'y rapporte.
pub(crate) fn writable<'a>(systems: &[&'a System]) -> Vec<&'a Choice> {
    systems
        .iter()
        .flat_map(|system| &system.choices)
        .filter(|choice| matches!(choice.source, ChoiceSource::Download { .. }))
        .collect()
}

/// Le système et la version dont `url` est l'adresse.
pub(crate) fn find_url<'a>(systems: &'a [System], url: &str) -> Option<(&'a System, &'a Choice)> {
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

/// Le bouton fermé du menu : le choix en cours avec son logo, ou l'invitation à choisir.
pub(crate) fn menu_button<'a>(
    state: &'a CatalogState,
    image_url: &str,
    open: bool,
    lang: Lang,
) -> Element<'a, Message> {
    let chosen = match state {
        CatalogState::Ready(systems) => find_url(systems, image_url),
        _ => None,
    };
    let content: Element<'a, Message> = match (state, chosen) {
        (_, Some((system, choice))) => row![
            logo(system.icon.as_ref()),
            text(format!("{} {}", choice.name, choice.title(lang))).size(14),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into(),
        (CatalogState::Loading | CatalogState::NotLoaded, _) => text(lang.pick(
            "Reading the list of systems...",
            "Lecture de la liste des systèmes...",
        ))
        .size(14)
        .into(),
        (CatalogState::Failed(_), _) => text(lang.pick(
            "List of systems unavailable",
            "Liste des systèmes indisponible",
        ))
        .size(14)
        .into(),
        (CatalogState::Ready(_), None) => {
            text(lang.pick("Choose a system...", "Choisir un système..."))
                .size(14)
                .into()
        }
    };
    let arrow = text(if open { "^" } else { "v" }).size(14);
    button(
        row![content, iced::widget::space::horizontal(), arrow]
            .spacing(8)
            .align_y(iced::Center),
    )
    .on_press_maybe(matches!(state, CatalogState::Ready(_)).then_some(Message::ToggleMenu))
    .width(Length::Fill)
    .padding([8, 12])
    .style(theme::card_button(false))
    .into()
}

fn without_doubles(mut values: Vec<String>, first: &str) -> Vec<String> {
    values.sort();
    values.dedup();
    values.insert(0, first.to_owned());
    values
}

/// Les deux sélecteurs : une catégorie, ou un seul système et ses versions.
fn menu_filters<'a>(systems: &[System], filter: &MenuFilter, lang: Lang) -> Element<'a, Message> {
    let (all_cat, all_sys) = (all_categories(lang), all_systems(lang));
    let categories = without_doubles(
        systems.iter().map(|s| s.category.clone()).collect(),
        all_cat,
    );
    let in_category = MenuFilter {
        category: filter.category.clone(),
        system: None,
    };
    let names = without_doubles(
        in_category
            .apply(systems)
            .iter()
            .map(|s| s.name.clone())
            .collect(),
        all_sys,
    );
    let category = filter
        .category
        .clone()
        .unwrap_or_else(|| all_cat.to_owned());
    let system = filter.system.clone().unwrap_or_else(|| all_sys.to_owned());
    row![
        pick_list(categories, Some(category), move |value: String| {
            Message::MenuCategory((value != all_cat).then_some(value))
        })
        .text_size(13),
        pick_list(names, Some(system), move |value: String| {
            Message::MenuSystem((value != all_sys).then_some(value))
        })
        .text_size(13),
    ]
    .spacing(8)
    .into()
}

/// La liste ouverte : les filtres, puis une ligne par version, avec le logo de son système.
pub(crate) fn menu_list<'a>(
    systems: &'a [System],
    filter: &MenuFilter,
    highlighted: usize,
    lang: Lang,
) -> Element<'a, Message> {
    let mut index = 0;
    let mut lines = column![].spacing(2);
    for system in filter.apply(systems) {
        for choice in &system.choices {
            let title = format!("{} {}", choice.name, choice.title(lang));
            let line = row![logo(system.icon.as_ref()), text(title).size(14)]
                .spacing(8)
                .align_y(iced::Center);
            let line = match &choice.note {
                Some(note) => Element::from(column![
                    line,
                    text(note.as_str()).size(12).color(theme::SECONDARY)
                ]),
                None => line.into(),
            };
            lines = lines.push(match &choice.source {
                ChoiceSource::Download { .. } => {
                    let selected = index == highlighted;
                    index += 1;
                    Element::from(
                        button(line)
                            .on_press(Message::MenuChose(choice.clone()))
                            .width(Length::Fill)
                            .padding([4, 8])
                            .style(theme::card_button(selected)),
                    )
                }
                ChoiceSource::OfficialPage(page) => {
                    let note = text(match lang {
                        Lang::En => format!("download from {page}, then Other image"),
                        Lang::Fr => format!("à télécharger sur {page}, puis Autre image"),
                    })
                    .size(12)
                    .color(theme::SECONDARY);
                    container(column![line, note]).padding([4, 8]).into()
                }
                ChoiceSource::NotForUsb => container(line).padding([4, 8]).into(),
            });
        }
    }
    // Marge à droite : la barre de défilement ne doit pas couvrir le texte d'une ligne.
    column![
        menu_filters(systems, filter, lang),
        scrollable(lines.padding(iced::Padding::ZERO.right(16)))
            .id(MENU)
            .height(Length::Fixed(260.0)),
    ]
    .spacing(6)
    .into()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn report() -> Value {
        json!({
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
        })
    }

    #[test]
    fn systemes_avec_logo_et_versions_dans_l_ordre_recu() {
        let systems = parse(&report());
        assert_eq!(systems.len(), 2, "un système sans image n'est pas montré");
        assert_eq!(systems[0].icon, Some(PathBuf::from("/c/u.png")));
        assert_eq!(systems[0].choices[0].label, "26.04.1 desktop LTS");
        assert_eq!(
            systems[0].choices[0].name, "Ubuntu",
            "sans nom : celui du système"
        );
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
