//! Le menu des images à l'écran : la fenêtre lit `bonk catalog list --json`, comme elle écrit par `bonk write`.
//! Signature et expiration restent donc contrôlées au même endroit ; un choix donne l'adresse et l'empreinte
//! officielles, sans rien à régler.

use std::path::PathBuf;

use bonk_hal::Lang;
use iced::widget::{button, column, container, image, pick_list, row, scrollable, text};
use iced::{Element, Length};

pub(crate) use bonk_ui::catalog::*;

use crate::theme;

use super::Message;

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
        .map(|report| bonk_ui::catalog::systems(&report))
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
