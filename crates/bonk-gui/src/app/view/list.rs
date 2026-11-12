use bonk_hal::{Disk, Eligibility};
use iced::widget::{Column, column, container, scrollable, text};
use iced::{Element, Fill};

use bonk_ui::errors::technical;

use crate::theme;

use super::super::navigation::LIST;
use super::super::{App, Message};

impl App {
    /// Or what stands for it: detection error, empty list. `panels`: the write and vault panels under
    /// their disk; without, they are drawn beside the list.
    pub(super) fn disk_list<'a>(
        &'a self,
        entries: Vec<(&'a Disk, Eligibility)>,
        panels: bool,
    ) -> Element<'a, Message> {
        if let Some(erreur) = &self.error {
            self.detection_error(erreur)
        } else if entries.is_empty() {
            container(text(self.empty_line()).size(14).color(theme::muted()))
                .padding(12)
                .into()
        } else {
            let mut cards: Vec<Element<'_, Message>> = Vec::new();
            for (disk, verdict) in entries {
                cards.push(self.card(disk, verdict));
                let selected = self.selected.as_deref() == Some(disk.id.as_str());
                if selected {
                    cards.push(self.details_toggle());
                }
                // The write panel is outside the card: a click in it does not close the disk.
                if !panels {
                    continue;
                }
                if selected && let Some(panel) = self.write_panel(disk) {
                    cards.push(panel);
                }
                if let Some(panel) = self.vault_panel(disk) {
                    cards.push(panel);
                }
            }
            scrollable(Column::with_children(cards).spacing(8).width(Fill))
                .id(LIST)
                .height(Fill)
                .into()
        }
    }

    fn detection_error<'a>(&'a self, erreur: &'a str) -> Element<'a, Message> {
        let lang = self.lang;
        container(
            column![
                text(lang.pick(
                    "ERROR: the disks could not be detected.",
                    "ERREUR : les disques n'ont pas pu être détectés."
                ))
                .size(16),
                text(technical(lang, erreur)).size(14).color(theme::muted()),
                text(lang.pick(
                    "No disk is offered while detection fails.",
                    "Aucun disque n'est proposé tant que la détection échoue."
                ))
                .size(14)
                .color(theme::muted()),
            ]
            .spacing(6),
        )
        .padding(12)
        .style(theme::panel)
        .width(Fill)
        .into()
    }

    fn empty_line(&self) -> &'static str {
        let lang = self.lang;
        if self.loading && !self.answered {
            lang.pick("Detecting...", "Détection en cours...")
        } else if self.show_all {
            lang.pick("No disk detected.", "Aucun disque détecté.")
        } else {
            lang.pick(
                "No disk offered. Plug in a USB drive or an SD card. Settings can also show the disks this machine excludes, and why.",
                "Aucun disque proposé. Branchez une clé USB ou une carte SD. Les paramètres peuvent aussi montrer les disques que cette machine écarte, et pourquoi.",
            )
        }
    }
}
