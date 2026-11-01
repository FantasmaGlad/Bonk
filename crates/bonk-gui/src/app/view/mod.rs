//! Rendu de la fenêtre : liste, cartes, détail, panneau d'écriture et demande de confirmation.

use bonk_hal::{Disk, Eligibility, Lang};
use iced::widget::{
    Column, button, center, checkbox, column, container, opaque, row, scrollable, stack, text,
};
use iced::{Element, Fill};

use crate::theme;

use super::navigation::LIST;
use super::{App, Focus, Message, REFRESH_EVERY};

mod disk;
mod panel;

#[cfg(test)]
pub(super) use panel::secondes_restantes;

pub(super) use disk::internal_dialog;

/// Entoure un composant de l'indicateur de focus (visible seulement s'il a le focus).
pub(super) fn ring<'a>(
    content: impl Into<Element<'a, Message>>,
    focused: bool,
) -> Element<'a, Message> {
    container(content)
        .padding(6)
        .style(theme::focus_ring(focused))
        .into()
}

pub(super) fn boxed<'a>(body: Element<'a, Message>) -> Element<'a, Message> {
    container(body)
        .padding(12)
        .width(Fill)
        .style(theme::panel)
        .into()
}

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let entries = self.entries();
        let shown = entries.len();

        let header = row![
            column![
                text("BONK").size(28),
                text(lang.pick(
                    "Disk detection. A write needs an offered disk, a summary and three seconds to think.",
                    "Détection des disques. Une écriture exige un disque proposé, un récapitulatif et trois secondes de réflexion.",
                ))
                .size(14)
                .color(theme::SECONDARY),
            ]
            .spacing(4)
            .width(Fill),
            // Chaque langue est nommée dans sa propre langue : elle reste trouvable sans lire l'autre.
            ring(
                button(text(lang.pick("Français", "English")).size(14))
                    .on_press(Message::SwitchLanguage)
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::Language),
            ),
        ]
        .align_y(iced::Center);

        let controls = self.controls();

        let body = match self.active_panel() {
            Some(panel) => column![panel, self.disk_list(entries)].spacing(8).into(),
            None => self.disk_list(entries),
        };

        let equivalent = row![
            column![
                text(lang.pick("Equivalent command", "Commande équivalente"))
                    .size(13)
                    .color(theme::SECONDARY),
                text(self.equivalents().join("\n"))
                    .size(13)
                    .font(iced::Font::MONOSPACE),
            ]
            .width(Fill),
            ring(
                button(text(lang.pick("Copy", "Copier")).size(14))
                    .on_press(Message::Copy)
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.focus == Some(Focus::Copy),
            ),
        ]
        .align_y(iced::Center);

        let window = container(
            column![
                header,
                controls,
                text(self.status_line(shown))
                    .size(13)
                    .color(theme::SECONDARY),
                body,
                equivalent,
            ]
            .spacing(14),
        )
        .padding(20)
        .width(Fill)
        .height(Fill);
        match self.internal_to_confirm() {
            Some(disk) => stack![window, opaque(center(internal_dialog(disk, lang)))].into(),
            None => window.into(),
        }
    }

    pub(super) fn status_line(&self, shown: usize) -> String {
        let every = REFRESH_EVERY.as_secs();
        let detected = self.disks.len();
        if self.error.is_some() {
            return match self.lang {
                Lang::En => format!("Detection failed. Automatic retry every {every} s."),
                Lang::Fr => {
                    format!(
                        "Détection en échec. Nouvelle tentative automatique toutes les {every} s."
                    )
                }
            };
        }
        if self.loading && self.disks.is_empty() {
            return self
                .lang
                .pick("Detecting...", "Détection en cours...")
                .to_owned();
        }
        match self.lang {
            Lang::En => format!(
                "{shown} of {detected} detected disk(s) shown. Refreshes automatically every {every} s."
            ),
            Lang::Fr => format!(
                "{shown} disque(s) affiché(s) sur {detected} détecté(s). Actualisation automatique toutes les {every} s."
            ),
        }
    }

    fn controls(&self) -> Element<'_, Message> {
        let lang = self.lang;
        row![
            ring(
                button(text(lang.pick("Refresh", "Actualiser")).size(14))
                    .on_press(Message::Refresh)
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::Refresh),
            ),
            ring(
                checkbox(self.show_internal)
                    .label(lang.pick("Internal non-system disks", "Disques internes non système"))
                    .on_toggle(Message::ToggleInternal)
                    .size(18)
                    .style(theme::checkbox_style),
                self.has_focus(Focus::Internal),
            ),
            ring(
                checkbox(self.allow_virtual)
                    .label(lang.pick("Virtual disks (test mode)", "Disques virtuels (mode test)"))
                    .on_toggle(Message::ToggleVirtual)
                    .size(18)
                    .style(theme::checkbox_style),
                self.has_focus(Focus::Virtual),
            ),
            ring(
                checkbox(self.show_all)
                    .label(lang.pick(
                        "Also show excluded disks",
                        "Afficher aussi les disques écartés"
                    ))
                    .on_toggle(Message::ToggleAll)
                    .size(18)
                    .style(theme::checkbox_style),
                self.has_focus(Focus::All),
            ),
        ]
        .spacing(12)
        .align_y(iced::Center)
        .into()
    }

    /// La liste des disques, ou ce qui en tient lieu : erreur de détection, liste vide.
    fn disk_list<'a>(&'a self, entries: Vec<(&'a Disk, Eligibility)>) -> Element<'a, Message> {
        let lang = self.lang;
        if let Some(erreur) = &self.error {
            container(
                column![
                    text(lang.pick(
                        "ERROR: the disks could not be detected.",
                        "ERREUR : les disques n'ont pas pu être détectés."
                    ))
                    .size(16),
                    text(erreur.as_str()).size(14).color(theme::SECONDARY),
                    text(lang.pick(
                        "No disk is offered while detection fails.",
                        "Aucun disque n'est proposé tant que la détection échoue."
                    ))
                    .size(14)
                    .color(theme::SECONDARY),
                ]
                .spacing(6),
            )
            .padding(12)
            .style(theme::panel)
            .width(Fill)
            .into()
        } else if entries.is_empty() {
            let message = if self.loading {
                lang.pick("Detecting...", "Détection en cours...")
            } else if self.show_all {
                lang.pick("No disk detected.", "Aucun disque détecté.")
            } else {
                lang.pick(
                    "No disk offered. Plug in a USB drive or an SD card, or tick \"Also show excluded disks\" to see why this machine's disks are excluded.",
                    "Aucun disque proposé. Branchez une clé USB ou une carte SD, ou cochez « Afficher aussi les disques écartés » pour voir pourquoi les disques de cette machine sont écartés.",
                )
            };
            container(text(message).size(14).color(theme::SECONDARY))
                .padding(12)
                .into()
        } else {
            let mut cards: Vec<Element<'_, Message>> = Vec::new();
            for (disk, verdict) in entries {
                cards.push(self.card(disk, verdict));
                // Le panneau d'écriture est hors de la carte : un clic dedans ne referme pas le disque.
                if self.selected.as_deref() == Some(disk.id.as_str())
                    && let Some(panel) = self.write_panel(disk)
                {
                    cards.push(panel);
                }
            }
            scrollable(Column::with_children(cards).spacing(4).width(Fill))
                .id(LIST)
                .height(Fill)
                .into()
        }
    }

    pub(super) fn has_focus(&self, target: Focus) -> bool {
        self.focus.as_ref() == Some(&target)
    }
}
