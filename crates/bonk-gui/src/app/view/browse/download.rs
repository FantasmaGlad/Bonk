use iced::Element;
use iced::widget::{button, column, progress_bar, text};

use bonk_ui::browse::{BrowseMessage, DownloadState, Step};
use bonk_ui::catalog::System;
use bonk_ui::text as phrases;

use crate::theme;

use super::super::super::{App, Focus, Message};
use super::super::{boxed, ring};

impl App {
    /// Over the variants of the system it belongs to.
    pub(super) fn download_panel<'a>(&'a self, system: &System) -> Option<Element<'a, Message>> {
        let download = self.browse.download.as_ref()?;
        let choice = system.choices.iter().find(|c| c.id == download.id())?;
        let lang = self.lang;
        let title = text(format!("{} {}", choice.name, choice.title(lang))).size(14);
        let (lines, label, message): (Element<'a, Message>, _, _) = match download {
            DownloadState::Running { step, .. } => (
                self.download_progress(*step),
                lang.pick("Cancel", "Annuler"),
                BrowseMessage::CancelDownload,
            ),
            DownloadState::Finished { outcome, .. } => (
                text(phrases::outcome_with_explain(lang, outcome))
                    .size(13)
                    .into(),
                lang.pick("Close", "Fermer"),
                BrowseMessage::DismissDownload,
            ),
        };
        let action = ring(
            button(text(label).size(13))
                .on_press(Message::Browse(message))
                .padding([6, 12])
                .style(theme::primary_button),
            self.has_focus(Focus::DownloadAction),
        );
        Some(boxed(column![title, lines, action].spacing(8).into()))
    }

    fn download_progress<'a>(&self, step: Option<Step>) -> Element<'a, Message> {
        let lang = self.lang;
        let Some(step) = step else {
            return text(lang.pick("Starting the download...", "Début du téléchargement..."))
                .size(13)
                .into();
        };
        let (line, fraction) =
            phrases::progress(lang, step.phase, step.done, step.total, step.rate);
        column![
            text(line).size(13),
            progress_bar(0.0..=1.0, fraction).girth(10)
        ]
        .spacing(6)
        .into()
    }
}
