use iced::widget::{button, column, row, text};
use iced::{Element, Fill};

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

impl App {
    /// The command lines that do what the window shows, to copy.
    pub(super) fn command_bar(&self) -> Element<'_, Message> {
        let lang = self.lang;
        row![
            column![
                text(lang.pick("Equivalent command", "Commande équivalente"))
                    .size(13)
                    .color(theme::muted()),
                text(self.equivalents().join("\n"))
                    .size(13)
                    .font(theme::MONO),
            ]
            .width(Fill),
            ring(
                button(text(lang.pick("Copy", "Copier")).size(14))
                    .on_press(Message::Copy)
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.focus == Some(Focus::Copy),
            ),
        ]
        .align_y(iced::Center)
        .into()
    }

    /// Nothing when the window was not opened for an image, or once a draft carries it.
    pub(super) fn requested_notice(&self) -> Option<Element<'_, Message>> {
        let disk_chosen = matches!(self.write, super::super::WriteState::Draft { .. });
        let (id, state) = self.requested()?;
        bonk_ui::text::requested_line(self.lang, id, state, disk_chosen)
            .map(|line| text(line).size(14).into())
    }
}
