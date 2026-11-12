use std::path::PathBuf;

use bonk_hal::Lang;
use iced::widget::{button, column, container, image, pick_list, row, scrollable, text};
use iced::{Element, Length};

pub(crate) use bonk_ui::catalog::*;

use bonk_ui::uses;

use crate::theme;

use super::Message;

/// In a separate process. The last error line says why it failed.
pub(crate) fn read(program: PathBuf) -> Result<Vec<System>, String> {
    let output = run_json(&program, &["catalog", "list", "--json"])?;
    serde_json::from_slice(&output)
        .map(|report| bonk_ui::catalog::systems(&report))
        .map_err(|e| e.to_string())
}

/// Standard output of a `bonk` command, or its last error line.
pub(crate) fn run_json(program: &std::path::Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut command = std::process::Command::new(program);
    bonk_hal::console::hide_child_console(&mut command);
    let output = command
        .args(args)
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
    Ok(output.stdout)
}

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

/// The current choice with its logo, or an invitation to choose.
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

fn with_all(mut values: Vec<String>, all: &str) -> Vec<String> {
    values.insert(0, all.to_owned());
    values
}

/// A category, or a single system and its versions.
fn menu_filters<'a>(systems: &[System], filter: &MenuFilter, lang: Lang) -> Element<'a, Message> {
    let (all_cat, all_sys) = (all_categories(lang), all_systems(lang));
    let (categories, names) = filter.lists(systems);
    let labels = categories.iter().map(|id| uses::label(id, lang)).collect();
    let (categories, names) = (with_all(labels, all_cat), with_all(names, all_sys));
    let category = filter
        .category
        .as_deref()
        .map_or_else(|| all_cat.to_owned(), |id| uses::label(id, lang));
    let system = filter.system.clone().unwrap_or_else(|| all_sys.to_owned());
    row![
        pick_list(categories, Some(category), move |value: String| {
            Message::MenuCategory(uses::id_of(&value, lang))
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

/// The filters, then one row per version with its system logo.
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
                    text(note.as_str()).size(12).color(theme::muted())
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
                    let note = text(official_page_note(page, lang))
                        .size(12)
                        .color(theme::muted());
                    container(column![line, note]).padding([4, 8]).into()
                }
                ChoiceSource::NotForUsb => container(line).padding([4, 8]).into(),
            });
        }
    }
    // Right margin: the scrollbar must not cover a row's text.
    column![
        menu_filters(systems, filter, lang),
        scrollable(lines.padding(iced::Padding::ZERO.right(16)))
            .id(MENU)
            .height(Length::Fixed(260.0)),
    ]
    .spacing(6)
    .into()
}
