use bonk_hal::{Disk, Eligibility, Lang, format_size, state_label, verdict_badge};
use bonk_ui::disk as disk_text;
use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill};

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

pub(in super::super) fn internal_dialog(disk: &Disk, lang: Lang) -> Element<'_, Message> {
    container(
        column![
            text(lang.pick("WARNING: INTERNAL DISK", "ATTENTION : DISQUE INTERNE")).size(18),
            text(format!(
                "{} ({}, {}) {}",
                disk.path,
                disk.model,
                format_size(disk.size_bytes, lang),
                lang.pick(
                    "is part of this machine. All its data will be erased, with no way back.",
                    "fait partie de cette machine. Toutes ses données seront effacées, sans retour possible.",
                ),
            ))
            .size(14),
            row![
                button(text(lang.pick("Give up", "Renoncer")).size(14))
                    .on_press(Message::CloseInternal)
                    .padding([6, 14])
                    .style(theme::primary_button),
                button(text(lang.pick("Erase this internal disk", "Effacer ce disque interne")).size(14))
                    .on_press(Message::ConfirmInternal)
                    .padding([6, 14])
                    .style(theme::primary_button),
            ]
            .spacing(12),
        ]
        .spacing(12)
        .max_width(520),
    )
    .padding(20)
    .style(theme::panel)
    .into()
}

impl App {
    /// What a person recognises: model, capacity, labels, verdict. The technical lines open on demand.
    pub(super) fn card<'a>(&'a self, disk: &'a Disk, verdict: Eligibility) -> Element<'a, Message> {
        let lang = self.lang;
        let selected = self.selected.as_deref() == Some(disk.id.as_str());
        let top = row![
            text(disk.model.as_str()).size(16).width(Fill),
            text(verdict_badge(&verdict, lang)).size(14),
        ];
        let mut content = column![top, text(disk_text::facts(lang, disk)).size(13)].spacing(4);
        if !matches!(
            &verdict,
            Eligibility::Offered {
                warning: bonk_hal::Warning::None
            }
        ) {
            content = content.push(
                text(state_label(&verdict, lang))
                    .size(13)
                    .color(theme::muted()),
            );
        }
        if selected && self.details {
            for line in disk_text::details(lang, disk) {
                content = content.push(text(line).size(12).font(theme::MONO).color(theme::muted()));
            }
        }
        let focused = self.focus == Some(Focus::Disk(disk.id.clone()));
        button(content)
            .on_press(Message::Select(disk.id.clone()))
            .padding(12)
            .width(Fill)
            .style(theme::card(selected, focused))
            .into()
    }

    /// Under the selected card, outside it: one control never sits inside another.
    pub(super) fn details_toggle(&self) -> Element<'_, Message> {
        ring(
            button(text(disk_text::details_button(self.lang, self.details)).size(13))
                .on_press(Message::ToggleDetails)
                .padding([2, 8])
                .style(theme::link_button),
            self.focus == Some(Focus::Details),
        )
    }
}
