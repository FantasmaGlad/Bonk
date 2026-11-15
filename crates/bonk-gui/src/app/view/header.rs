use iced::widget::{Row, button, column, text};
use iced::{Element, Fill};

use bonk_ui::browse::BrowseMessage;
use bonk_ui::library::LibraryMessage;
use bonk_ui::settings::SettingsMessage;

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

impl App {
    /// Title, then a quiet line on detection: it runs on its own every two seconds.
    pub(super) fn header(&self, shown: usize) -> Element<'_, Message> {
        let lang = self.lang;
        let title = column![
            text("BONK").size(28),
            text(self.status_line(shown)).size(13).color(theme::muted()),
        ]
        .spacing(2)
        .width(Fill);
        Row::new()
            .push(title)
            .extend(self.incognito_control())
            .extend(self.catalog_button())
            .push(ring(
                button(text(lang.pick("Library", "Bibliothèque")).size(14))
                    .on_press(Message::Library(LibraryMessage::Open))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::Library),
            ))
            .push(ring(
                button(text(lang.pick("Settings", "Paramètres")).size(14))
                    .on_press(Message::Settings(SettingsMessage::Open))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::Settings),
            ))
            .spacing(8)
            .align_y(iced::Center)
            .into()
    }

    /// Hidden offline: the catalog is never read there.
    fn catalog_button(&self) -> Option<Element<'_, Message>> {
        (!self.offline).then(|| {
            ring(
                button(text(self.lang.pick("Catalog", "Catalogue")).size(14))
                    .on_press(Message::Browse(BrowseMessage::Open))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::Catalog),
            )
        })
    }
}
