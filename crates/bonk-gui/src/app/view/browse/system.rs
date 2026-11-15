use iced::widget::{Column, button, column, row, scrollable, text};
use iced::{Element, Fill};

use bonk_hal::Lang;
use bonk_ui::browse::{BrowseMessage, desktop_label, media_label};
use bonk_ui::catalog::{Choice, ChoiceSource, System, official_page_note};

use crate::app::catalog::sized_logo;
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
            .filter(|choice| self.browse.filter.keeps_choice(choice))
            .map(|choice| self.variant_row(choice));
        let list =
            scrollable(Column::with_children(variants).spacing(8).padding([0, 4])).height(Fill);
        column![back, head, list].spacing(12).into()
    }

    fn variant_row<'a>(&'a self, choice: &'a Choice) -> Element<'a, Message> {
        let lang = self.lang;
        let about = column![
            text(format!("{} {}", choice.name, choice.title(lang))).size(14),
            text(facts(choice, lang)).size(12).color(theme::muted()),
        ]
        .spacing(2)
        .width(Fill);
        let action: Element<'a, Message> = match &choice.source {
            ChoiceSource::Download { .. } => ring(
                button(text(lang.pick("Write this image", "Écrire cette image")).size(13))
                    .on_press(Message::Browse(BrowseMessage::Write(choice.id.clone())))
                    .padding([6, 12])
                    .style(theme::primary_button),
                self.has_focus(Focus::Variant(choice.id.clone())),
            ),
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
}

/// Desktop, medium and the catalog note, on one line.
fn facts(choice: &Choice, lang: Lang) -> String {
    [
        choice.desktop.as_deref().map(|id| desktop_label(id, lang)),
        choice.media.as_deref().map(|id| media_label(id, lang)),
        choice.note.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}
