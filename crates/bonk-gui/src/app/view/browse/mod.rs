mod download;
mod filters;
mod system;

use iced::widget::{Row, button, column, row, scrollable, text, text_input};
use iced::{Element, Fill, Length};

use bonk_ui::browse::{BrowseMessage, writable_count};
use bonk_ui::catalog::{CatalogState, System};

use crate::app::catalog::sized_logo;
use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

const CARD_WIDTH: f32 = 168.0;
const CARD_LOGO: f32 = 48.0;

impl App {
    pub(super) fn catalog_page(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let title = row![
            text(lang.pick("Catalog", "Catalogue")).size(28).width(Fill),
            ring(
                button(text(lang.pick("Back", "Retour")).size(14))
                    .on_press(Message::Browse(BrowseMessage::Close))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::CloseCatalog),
            ),
        ]
        .align_y(iced::Center);
        let body = match (&self.catalog, self.opened_system()) {
            (CatalogState::Ready(_), Some(system)) => self.system_page(system),
            (CatalogState::Ready(systems), None) => self.catalog_grid(systems),
            (CatalogState::Failed(error), _) => quiet(error.clone()),
            _ => quiet(
                lang.pick(
                    "Reading the list of systems...",
                    "Lecture de la liste des systèmes...",
                )
                .to_owned(),
            ),
        };
        column![title, body].spacing(14).into()
    }

    fn catalog_grid<'a>(&'a self, systems: &'a [System]) -> Element<'a, Message> {
        let lang = self.lang;
        let search = ring(
            text_input(
                lang.pick(
                    "Search: Ubuntu, Fedora, Windows…",
                    "Rechercher : Ubuntu, Fedora, Windows…",
                ),
                &self.browse.filter.search,
            )
            .on_input(|value| Message::Browse(BrowseMessage::Search(value)))
            .padding(8)
            .size(14)
            .style(theme::input_style),
            self.has_focus(Focus::CatalogSearch),
        );
        let shown = self.browsed();
        let grid: Element<'a, Message> = if shown.is_empty() {
            quiet(
                lang.pick("No system matches.", "Aucun système ne correspond.")
                    .to_owned(),
            )
        } else {
            let cards = shown.into_iter().map(|system| self.system_card(system));
            scrollable(
                Row::with_children(cards)
                    .spacing(12)
                    .wrap()
                    .vertical_spacing(12),
            )
            .height(Fill)
            .into()
        };
        column![search, self.catalog_filters(systems), grid]
            .spacing(12)
            .into()
    }

    fn system_card<'a>(&'a self, system: &'a System) -> Element<'a, Message> {
        let count = writable_count(system, self.lang);
        let content = column![
            sized_logo(system.icon.as_ref(), CARD_LOGO),
            text(system.name.as_str()).size(15),
            text(count).size(12).color(theme::muted()),
        ]
        .spacing(6);
        let focused = self.has_focus(Focus::System(system.name.clone()));
        button(content)
            .on_press(Message::Browse(BrowseMessage::OpenSystem(
                system.name.clone(),
            )))
            .width(Length::Fixed(CARD_WIDTH))
            .padding(12)
            .style(theme::card(false, focused))
            .into()
    }
}

fn quiet<'a>(line: String) -> Element<'a, Message> {
    text(line).size(14).color(theme::muted()).into()
}
