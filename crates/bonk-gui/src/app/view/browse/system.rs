use iced::widget::{Column, button, column, row, scrollable, text};
use iced::{Element, Fill};

use bonk_ui::browse::{BrowseMessage, DownloadState, variant_facts};
use bonk_ui::catalog::{Choice, ChoiceSource, System, official_page_note};

use crate::app::view::menu::sized_logo;
use crate::theme;

use super::super::super::{App, Focus, Message};
use super::super::ring;

const PAGE_LOGO: f32 = 64.0;

impl App {
    pub(super) fn system_page<'a>(&'a self, system: &'a System) -> Element<'a, Message> {
        let lang = self.lang;
        let back = ring(
            button(text(lang.pick("All systems", "Tous les systèmes")).size(14))
                .on_press(Message::Browse(BrowseMessage::CloseSystem))
                .padding([6, 14])
                .style(theme::link_button),
            self.has_focus(Focus::CloseSystem),
        );
        let head = row![
            sized_logo(system.icon.as_ref(), PAGE_LOGO),
            text(system.name.as_str()).size(24)
        ]
        .spacing(12)
        .align_y(iced::Center);
        let variants = system
            .choices
            .iter()
            .filter(|choice| self.browse.filter.keeps_choice(system, choice))
            .map(|choice| self.variant_row(choice));
        let list =
            scrollable(Column::with_children(variants).spacing(8).padding([0, 4])).height(Fill);
        let mut page = column![back, head].spacing(12);
        if let Some(panel) = self.download_panel(system) {
            page = page.push(panel);
        }
        page.push(list).into()
    }

    fn variant_row<'a>(&'a self, choice: &'a Choice) -> Element<'a, Message> {
        let lang = self.lang;
        let about = column![
            text(format!("{} {}", choice.name, choice.title(lang))).size(14),
            text(variant_facts(choice, lang))
                .size(12)
                .color(theme::muted()),
        ]
        .spacing(2)
        .width(Fill);
        let action: Element<'a, Message> = match &choice.source {
            ChoiceSource::Download { .. } => self.variant_buttons(&choice.id),
            ChoiceSource::OfficialPage(page) => text(official_page_note(page, lang))
                .size(12)
                .color(theme::muted())
                .into(),
            ChoiceSource::NotForUsb => text(lang.pick(
                "Does not start from a drive",
                "Ne démarre pas depuis une clé",
            ))
            .size(12)
            .color(theme::muted())
            .into(),
        };
        super::super::boxed(row![about, action].spacing(12).align_y(iced::Center).into())
    }

    /// Downloading waits while another download runs: one at a time.
    fn variant_buttons<'a>(&self, id: &str) -> Element<'a, Message> {
        let lang = self.lang;
        let busy = self
            .browse
            .download
            .as_ref()
            .is_some_and(DownloadState::running);
        let write = ring(
            button(text(lang.pick("Write this image", "Écrire cette image")).size(13))
                .on_press(Message::Browse(BrowseMessage::Write(id.to_owned())))
                .padding([6, 12])
                .style(theme::primary_button),
            self.has_focus(Focus::Variant(id.to_owned())),
        );
        let download = ring(
            button(text(lang.pick("Download", "Télécharger")).size(13))
                .on_press_maybe(
                    (!busy).then(|| Message::Browse(BrowseMessage::Download(id.to_owned()))),
                )
                .padding([6, 12])
                .style(theme::card_button(false)),
            self.has_focus(Focus::Download(id.to_owned())),
        );
        row![download, write].spacing(8).into()
    }
}
