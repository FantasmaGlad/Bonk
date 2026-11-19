use std::path::PathBuf;

use bonk_hal::Lang;
use iced::widget::{
    button, column, container, image, pick_list, row, scrollable, text, text_input,
};
use iced::{Element, Length};

use bonk_ui::catalog::*;
use bonk_ui::terms::Terms;

use crate::theme;

use super::super::Message;

pub(crate) const MENU: &str = "menu-images";

fn logo(icon: Option<&PathBuf>) -> Element<'_, Message> {
    sized_logo(icon, 24.0)
}

/// A system without a logo keeps the same width: names stay aligned.
pub(crate) fn sized_logo(icon: Option<&PathBuf>, side: f32) -> Element<'_, Message> {
    match icon {
        Some(path) => image(image::Handle::from_path(path))
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .into(),
        None => container(text(""))
            .width(Length::Fixed(side))
            .height(Length::Fixed(side))
            .into(),
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

fn arch_filter<'a>(arches: Vec<String>, chosen: Option<&str>, lang: Lang) -> Element<'a, Message> {
    let all = all_arches(lang);
    let names = with_all(arches.iter().map(|a| arch_label(a, lang)).collect(), all);
    let shown = chosen.map_or_else(|| all.to_owned(), |a| arch_label(a, lang));
    pick_list(names, Some(shown), move |value: String| {
        Message::MenuArch(
            arches
                .iter()
                .find(|id| arch_label(id, lang) == value)
                .cloned(),
        )
    })
    .text_size(13)
    .into()
}

/// A category, a base, or a single system and its versions.
fn menu_filters<'a>(
    systems: &[System],
    filter: &MenuFilter,
    terms: &Terms,
    lang: Lang,
) -> Element<'a, Message> {
    let (all_cat, all_base, all_sys) = (all_categories(lang), all_bases(lang), all_systems(lang));
    let lists = filter.lists(systems, terms);
    let (uses, bases) = (terms.uses(), terms.bases());
    let labels = |ids: &[String], list: bonk_ui::terms::List<'_>, all: &str| {
        with_all(ids.iter().map(|id| list.label(id, lang)).collect(), all)
    };
    let shown = |id: Option<&str>, list: bonk_ui::terms::List<'_>, all: &str| {
        id.map_or_else(|| all.to_owned(), |id| list.label(id, lang))
    };
    let categories = labels(&lists.categories, uses, all_cat);
    let base_names = labels(&lists.bases, bases, all_base);
    let category = shown(filter.category.as_deref(), uses, all_cat);
    let base = shown(filter.base.as_deref(), bases, all_base);
    let names = with_all(lists.systems, all_sys);
    let system = filter.system.clone().unwrap_or_else(|| all_sys.to_owned());
    // The lists outlive this borrow of the window state: each choice keeps its own copy of the labels.
    let (for_uses, for_bases) = (terms.clone(), terms.clone());
    row![
        pick_list(categories, Some(category), move |value: String| {
            Message::MenuCategory(for_uses.uses().id_of(&value, lang))
        })
        .text_size(13),
        pick_list(base_names, Some(base), move |value: String| {
            Message::MenuBase(for_bases.bases().id_of(&value, lang))
        })
        .text_size(13),
        pick_list(names, Some(system), move |value: String| {
            Message::MenuSystem((value != all_sys).then_some(value))
        })
        .text_size(13),
        arch_filter(lists.arches, filter.arch.as_deref(), lang),
    ]
    .spacing(8)
    .into()
}

fn menu_search<'a>(filter: &MenuFilter, lang: Lang) -> Element<'a, Message> {
    text_input(
        lang.pick(
            "Search by name: Ubuntu, Kubuntu, Windows…",
            "Rechercher par nom : Ubuntu, Kubuntu, Windows…",
        ),
        &filter.search,
    )
    .on_input(Message::MenuSearch)
    .padding(6)
    .size(13)
    .into()
}

fn menu_row<'a>(
    system: &'a System,
    choice: &'a Choice,
    highlighted: bool,
    lang: Lang,
) -> Element<'a, Message> {
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
    match &choice.source {
        ChoiceSource::Download { .. } => button(line)
            .on_press(Message::MenuChose(choice.clone()))
            .width(Length::Fill)
            .padding([4, 8])
            .style(theme::card_button(highlighted))
            .into(),
        ChoiceSource::OfficialPage(page) => {
            let note = text(official_page_note(page, lang))
                .size(12)
                .color(theme::muted());
            container(column![line, note]).padding([4, 8]).into()
        }
        ChoiceSource::NotForUsb => container(line).padding([4, 8]).into(),
    }
}

pub(crate) fn menu_list<'a>(
    systems: &'a [System],
    filter: &MenuFilter,
    terms: &Terms,
    highlighted: usize,
    lang: Lang,
) -> Element<'a, Message> {
    // Only the writable rows count for the keyboard highlight.
    let mut index = 0;
    let mut lines = column![].spacing(2);
    for system in filter.apply(systems) {
        for choice in filter.choices(system) {
            let writable = matches!(choice.source, ChoiceSource::Download { .. });
            lines = lines.push(menu_row(
                system,
                choice,
                writable && index == highlighted,
                lang,
            ));
            index += usize::from(writable);
        }
    }
    // Right margin: the scrollbar must not cover a row's text.
    column![
        menu_search(filter, lang),
        menu_filters(systems, filter, terms, lang),
        scrollable(lines.padding(iced::Padding::ZERO.right(16)))
            .id(MENU)
            .height(Length::Fixed(260.0)),
    ]
    .spacing(6)
    .into()
}
