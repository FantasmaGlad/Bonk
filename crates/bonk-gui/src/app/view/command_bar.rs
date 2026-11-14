use iced::widget::{button, column, row, text};
use iced::{Element, Fill};

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

impl App {
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
        if let Some(bonk_ui::source::ImageSource::Kept { path, .. }) = &self.library.chosen {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let line = match self.lang {
                bonk_hal::Lang::En => format!("{name} is chosen: select the disk to write it to."),
                bonk_hal::Lang::Fr => {
                    format!("{name} est choisie : sélectionnez le disque où l'écrire.")
                }
            };
            return Some(text(line).size(14).into());
        }
        let disk_chosen = matches!(self.write, super::super::WriteState::Draft { .. });
        let (id, state) = self.requested()?;
        bonk_ui::text::requested_line(self.lang, id, state, disk_chosen)
            .map(|line| text(line).size(14).into())
    }
}
