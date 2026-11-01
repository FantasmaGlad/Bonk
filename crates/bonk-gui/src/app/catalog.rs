//! Le menu des images : la fenêtre lit `bonk catalog list --json`, comme elle écrit par `bonk write`. Signature
//! et expiration restent donc contrôlées au même endroit ; un choix donne l'adresse et l'empreinte officielles,
//! sans rien à régler.

use std::path::PathBuf;

use iced::widget::{button, column, container, image, row, scrollable, text};
use iced::{Element, Length};
use serde_json::Value;

use crate::theme;

use super::Message;

/// Une version proposée d'un système.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Choice {
    pub(crate) label: String,
    pub(crate) end_of_life: bool,
    pub(crate) source: ChoiceSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChoiceSource {
    Download {
        url: String,
        sha256: String,
    },
    /// Pas de lien stable chez l'éditeur : l'adresse de sa page, à ouvrir soi-même.
    OfficialPage(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct System {
    pub(crate) name: String,
    pub(crate) icon: Option<PathBuf>,
    pub(crate) choices: Vec<Choice>,
}

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
    match image["channel"].as_str() {
        Some("lts") => label.push_str(" LTS"),
        Some("end_of_life") => label.push_str(" (fin de vie)"),
        _ => {}
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
                        (Some(url), Some(sha256)) => ChoiceSource::Download {
                            url: url.to_owned(),
                            sha256: sha256.to_owned(),
                        },
                        _ => {
                            ChoiceSource::OfficialPage(image["official_page"].as_str()?.to_owned())
                        }
                    };
                    Some(Choice {
                        label: label_of(image)?,
                        end_of_life: image["channel"] == "end_of_life",
                        source,
                    })
                })
                .collect();
            (!choices.is_empty()).then(|| System {
                name: family["name"].as_str().unwrap_or(id).to_owned(),
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
        .map_err(|e| format!("{} : lancement impossible : {e}", program.display()))?;
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
pub(crate) fn writable(systems: &[System]) -> Vec<&Choice> {
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
) -> Element<'a, Message> {
    let chosen = match state {
        CatalogState::Ready(systems) => find_url(systems, image_url),
        _ => None,
    };
    let content: Element<'a, Message> = match (state, chosen) {
        (_, Some((system, choice))) => row![
            logo(system.icon.as_ref()),
            text(format!("{} {}", system.name, choice.label)).size(14),
        ]
        .spacing(8)
        .align_y(iced::Center)
        .into(),
        (CatalogState::Loading | CatalogState::NotLoaded, _) => {
            text("Lecture de la liste des systèmes...").size(14).into()
        }
        (CatalogState::Failed(_), _) => text("Liste des systèmes indisponible").size(14).into(),
        (CatalogState::Ready(_), None) => text("Choisir un système...").size(14).into(),
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

/// La liste ouverte : une ligne par version, avec le logo de son système.
pub(crate) fn menu_list<'a>(systems: &'a [System], highlighted: usize) -> Element<'a, Message> {
    let mut index = 0;
    let mut lines = column![].spacing(2);
    for system in systems {
        for choice in &system.choices {
            let title = format!("{} {}", system.name, choice.label);
            let line = row![logo(system.icon.as_ref()), text(title).size(14)]
                .spacing(8)
                .align_y(iced::Center);
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
                    let note = text(format!("à télécharger sur {page}, puis Autre image"))
                        .size(12)
                        .color(theme::SECONDARY);
                    container(column![line, note]).padding([4, 8]).into()
                }
            });
        }
    }
    // Marge à droite : la barre de défilement ne doit pas couvrir le texte d'une ligne.
    scrollable(lines.padding(iced::Padding::ZERO.right(16)))
        .id(MENU)
        .height(Length::Fixed(260.0))
        .into()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn report() -> Value {
        json!({
            "families": [
                {"id": "ubuntu", "name": "Ubuntu", "icon_path": "/c/u.png"},
                {"id": "vide", "name": "Vide"},
                {"id": "windows", "name": "Windows"}
            ],
            "images": [
                {"family": "ubuntu", "version": "26.04.1", "edition": "desktop", "channel": "lts",
                 "url": "https://u/26.iso", "sha256": "aa"},
                {"family": "ubuntu", "version": "20.04.5", "edition": "desktop", "channel": "end_of_life",
                 "url": "https://u/20.iso", "sha256": "bb"},
                {"family": "windows", "version": "11", "channel": "stable", "official_page": "https://p/"}
            ]
        })
    }

    #[test]
    fn systemes_avec_logo_et_versions_dans_l_ordre_recu() {
        let systems = parse(&report());
        assert_eq!(systems.len(), 2, "un système sans image n'est pas montré");
        assert_eq!(systems[0].icon, Some(PathBuf::from("/c/u.png")));
        assert_eq!(systems[0].choices[0].label, "26.04.1 desktop LTS");
        assert_eq!(systems[0].choices[1].label, "20.04.5 desktop (fin de vie)");
        assert!(systems[0].choices[1].end_of_life);
        assert_eq!(
            systems[1].choices[0].source,
            ChoiceSource::OfficialPage("https://p/".to_owned())
        );
    }

    #[test]
    fn le_menu_clavier_ne_compte_que_ce_qui_s_ecrit_et_retrouve_un_choix() {
        let systems = parse(&report());
        assert_eq!(writable(&systems).len(), 2);
        assert!(matches!(
            find_url(&systems, "https://u/20.iso"),
            Some((system, choice)) if system.name == "Ubuntu" && choice.end_of_life
        ));
        assert!(find_url(&systems, "https://ailleurs/").is_none());
    }

    #[test]
    fn rapport_illisible_rien_a_proposer() {
        assert!(parse(&json!({"autre": 1})).is_empty());
    }
}
