use iced::widget::{button, column, row, text};
use iced::{Element, Fill};

use bonk_ui::settings::SettingsMessage;

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

impl App {
    /// Title, then a quiet line on detection: it runs on its own every two seconds.
    pub(super) fn header(&self, shown: usize) -> Element<'_, Message> {
        let lang = self.lang;
        row![
            column![
                text("BONK").size(28),
                text(self.status_line(shown)).size(13).color(theme::muted()),
            ]
            .spacing(2)
            .width(Fill),
            ring(
                button(text(lang.pick("Settings", "Paramètres")).size(14))
                    .on_press(Message::Settings(SettingsMessage::Open))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::Settings),
            ),
        ]
        .align_y(iced::Center)
        .into()
    }
}
