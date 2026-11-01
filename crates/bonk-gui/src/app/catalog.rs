//! Choisir une image dans le catalogue : la fenêtre lit `bonk catalog list --json`, comme elle écrit par
//! `bonk write`. Signature et expiration restent donc contrôlées au même endroit ; un choix remplit simplement
//! l'adresse et l'empreinte, que l'utilisateur voit avant d'écrire.

use std::path::PathBuf;

use iced::widget::{button, column, container, row, scrollable, svg, text};
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
    Closed,
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

fn logo(icon: Option<&PathBuf>) -> Element<'_, Message> {
    match icon {
        Some(path) => svg(svg::Handle::from_path(path))
            .width(Length::Fixed(24.0))
            .height(Length::Fixed(24.0))
            .into(),
        None => container(text("")).width(Length::Fixed(24.0)).into(),
    }
}

fn version_button(choice: &Choice) -> Element<'_, Message> {
    match &choice.source {
        ChoiceSource::Download { .. } => {
            let chosen = button(text(choice.label.clone()).size(13))
                .on_press(Message::CatalogChose(choice.clone()))
                .padding([3, 8]);
            // Une version en fin de vie reste possible, sans être mise en avant.
            if choice.end_of_life {
                chosen.style(theme::card_button(false)).into()
            } else {
                chosen.style(theme::primary_button).into()
            }
        }
        ChoiceSource::OfficialPage(page) => {
            text(format!("{} : à télécharger sur {page}", choice.label))
                .size(13)
                .color(theme::SECONDARY)
                .into()
        }
    }
}

/// Un système par ligne : son logo, son nom, puis ses versions.
pub(crate) fn view(state: &CatalogState) -> Option<Element<'_, Message>> {
    match state {
        CatalogState::Closed => None,
        CatalogState::Loading => Some(text("Lecture du catalogue...").size(13).into()),
        CatalogState::Failed(error) => Some(
            text(format!("Catalogue indisponible : {error}"))
                .size(13)
                .color(theme::SECONDARY)
                .into(),
        ),
        CatalogState::Ready(systems) => {
            let lines = systems.iter().map(|system| {
                let versions = system.choices.iter().map(version_button);
                column![
                    row![
                        logo(system.icon.as_ref()),
                        text(system.name.clone()).size(14)
                    ]
                    .spacing(8)
                    .align_y(iced::Center),
                    row(versions).spacing(6).wrap(),
                ]
                .spacing(4)
                .into()
            });
            Some(
                // Marge à droite : la barre de défilement ne doit pas couvrir la dernière version d'une ligne.
                scrollable(
                    column(lines)
                        .spacing(10)
                        .padding(iced::Padding::ZERO.right(16)),
                )
                .height(Length::Fixed(240.0))
                .into(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn report() -> Value {
        json!({
            "families": [
                {"id": "ubuntu", "name": "Ubuntu", "icon_path": "/c/u.svg"},
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
        assert_eq!(systems[0].icon, Some(PathBuf::from("/c/u.svg")));
        assert_eq!(systems[0].choices[0].label, "26.04.1 desktop LTS");
        assert_eq!(systems[0].choices[1].label, "20.04.5 desktop (fin de vie)");
        assert!(systems[0].choices[1].end_of_life);
        assert_eq!(
            systems[1].choices[0].source,
            ChoiceSource::OfficialPage("https://p/".to_owned())
        );
    }

    #[test]
    fn rapport_illisible_rien_a_proposer() {
        assert!(parse(&json!({"autre": 1})).is_empty());
    }
}
