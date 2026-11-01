//! Un disque à l'écran : sa carte, son détail, la confirmation d'un disque interne.

use bonk_hal::{
    Disk, Eligibility, Lang, bus_label, format_size, kind_label, state_label, verdict_badge,
};
use iced::widget::{button, column, container, row, text};
use iced::{Element, Fill};

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

/// Demande de confirmation avant d'effacer un disque interne.
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

/// Récapitulatif d'un disque : série, nature, partitions, étiquettes, montages.
pub(in super::super) fn detail(disk: &Disk, lang: Lang) -> Element<'_, Message> {
    let mut lines = column![
        text(format!(
            "{} {}   {} {}   {} {}",
            lang.pick("Serial:", "Série :"),
            disk.serial
                .as_deref()
                .unwrap_or(lang.pick("unknown", "inconnue")),
            lang.pick("Kind:", "Nature :"),
            kind_label(disk.kind, lang),
            lang.pick("System removable flag:", "Drapeau amovible du système :"),
            if disk.removable_flag {
                lang.pick("yes", "oui")
            } else {
                lang.pick("no", "non")
            },
        ))
        .size(13)
        .color(theme::SECONDARY),
    ]
    .spacing(2);

    if disk.partitions.is_empty() {
        lines = lines.push(
            text(lang.pick("No partition detected.", "Aucune partition détectée."))
                .size(13)
                .color(theme::SECONDARY),
        );
    }
    for partition in &disk.partitions {
        let mounts = if partition.mount_points.is_empty() {
            lang.pick("not mounted", "non monté").to_owned()
        } else {
            format!(
                "{} {}",
                lang.pick("mounted on", "monté sur"),
                partition.mount_points.join(", ")
            )
        };
        lines = lines.push(
            text(format!(
                "{}  {}  {}  {} {}  {}",
                partition.path,
                format_size(partition.size_bytes, lang),
                partition
                    .fs_type
                    .as_deref()
                    .unwrap_or(lang.pick("unknown filesystem", "système de fichiers inconnu")),
                lang.pick("label:", "étiquette :"),
                partition
                    .label
                    .as_deref()
                    .unwrap_or(lang.pick("none", "aucune")),
                mounts,
            ))
            .size(13)
            .font(iced::Font::MONOSPACE),
        );
    }
    container(lines).padding([8, 0]).into()
}

impl App {
    pub(super) fn card<'a>(&'a self, disk: &'a Disk, verdict: Eligibility) -> Element<'a, Message> {
        let lang = self.lang;
        let selected = self.selected.as_deref() == Some(disk.id.as_str());

        let top = row![
            text(disk.model.as_str()).size(16).width(Fill),
            text(verdict_badge(&verdict, lang)).size(14),
        ];
        let facts = row![
            text(disk.path.as_str())
                .size(13)
                .font(iced::Font::MONOSPACE),
            text(format_size(disk.size_bytes, lang)).size(13),
            text(bus_label(disk.bus, lang)).size(13),
            text(disk.id.as_str()).size(13).font(iced::Font::MONOSPACE),
        ]
        .spacing(16);
        let mut content = column![top, facts.wrap()].spacing(4);
        if !matches!(
            &verdict,
            Eligibility::Offered {
                warning: bonk_hal::Warning::None
            }
        ) {
            content = content.push(
                text(state_label(&verdict, lang))
                    .size(13)
                    .color(theme::SECONDARY),
            );
        }
        if selected {
            content = content.push(detail(disk, lang));
        }

        let focused = self.focus == Some(Focus::Disk(disk.id.clone()));
        ring(
            button(content)
                .on_press(Message::Select(disk.id.clone()))
                .padding(12)
                .width(Fill)
                .style(theme::card_button(selected)),
            focused,
        )
    }
}
