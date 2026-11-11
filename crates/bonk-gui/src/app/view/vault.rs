use bonk_hal::{Disk, Lang, format_size};
use iced::Element;
use iced::widget::{button, column, progress_bar, row, text};

use bonk_ui::text as phrases;
use bonk_ui::vault::VaultAction;

use crate::theme;

use super::super::{App, Message, VaultMessage, VaultState};

fn action_button<'a>(
    label: &'a str,
    message: Option<VaultMessage>,
    primary: bool,
) -> Element<'a, Message> {
    let button = button(text(label).size(14))
        .on_press_maybe(message.map(Message::Vault))
        .padding([6, 14]);
    if primary {
        button.style(theme::primary_button).into()
    } else {
        button.style(theme::card_button(false)).into()
    }
}

fn busy_line(lang: Lang, action: VaultAction) -> &'static str {
    match action {
        VaultAction::List => lang.pick("Reading the drive...", "Lecture de la clé..."),
        VaultAction::Prepare => lang.pick("Preparing the vault...", "Préparation du coffre..."),
        VaultAction::Add => lang.pick("Adding the ISO...", "Ajout de l'ISO..."),
        VaultAction::Update => lang.pick(
            "Updating GRUB and the menu...",
            "Mise à jour de GRUB et du menu...",
        ),
    }
}

impl App {
    /// Below the card of the disk it reads: several ISO images on one drive, each started from a menu.
    pub(super) fn vault_panel<'a>(&'a self, disk: &'a Disk) -> Option<Element<'a, Message>> {
        if self.vault.disk_id() != Some(disk.id.as_str()) {
            return None;
        }
        let lang = self.lang;
        let title = text(lang.pick(
            "VAULT: SEVERAL ISO IMAGES ON ONE DRIVE",
            "COFFRE : PLUSIEURS IMAGES ISO SUR UNE CLÉ",
        ))
        .size(14);
        let close = action_button(
            lang.pick("Close", "Fermer"),
            Some(VaultMessage::Close),
            false,
        );
        let body: Element<'a, Message> = match &self.vault {
            VaultState::Closed => return None,
            VaultState::Busy {
                action,
                phase,
                done,
                total,
                ..
            } => {
                let mut panel = column![text(busy_line(lang, *action)).size(13)].spacing(8);
                if let Some(phase) = phase {
                    let (line, fraction) = phrases::progress(lang, *phase, *done, *total, 0.0);
                    panel = panel.push(text(line).size(13)).push(progress_bar(0.0..=1.0, fraction).girth(14));
                }
                if *action != VaultAction::List {
                    panel = panel.push(action_button(lang.pick("Cancel", "Annuler"), Some(VaultMessage::Cancel), true));
                }
                panel.into()
            }
            VaultState::NotVault { .. } => column![
                text(lang.pick(
                    "This disk is not a vault. Preparing it erases everything on it, then any ISO copied to it shows in a boot menu.",
                    "Ce disque n'est pas un coffre. Le préparer efface tout son contenu ; ensuite, chaque ISO copiée dessus apparaît dans un menu de démarrage.",
                ))
                .size(13),
                row![
                    action_button(lang.pick("Prepare as a vault...", "Préparer en coffre..."), Some(VaultMessage::Prepare), true),
                    close,
                ]
                .spacing(8),
            ]
            .spacing(8)
            .into(),
            VaultState::Confirming { .. } => column![
                text(phrases::disk_summary(lang, disk)).size(13),
                text(lang.pick(
                    "All the data on this disk will be erased. This cannot be undone.",
                    "Toutes les données de ce disque seront effacées. Cette action est irréversible.",
                ))
                .size(13)
                .color(theme::SECONDARY),
                row![
                    action_button(lang.pick("Erase and prepare", "Effacer et préparer"), Some(VaultMessage::ConfirmPrepare), true),
                    action_button(lang.pick("Cancel", "Annuler"), Some(VaultMessage::Open), false),
                ]
                .spacing(8),
            ]
            .spacing(8)
            .into(),
            VaultState::Ready { images, .. } => {
                let mut list = column![].spacing(2);
                if images.is_empty() {
                    list = list.push(text(lang.pick("No ISO image yet.", "Aucune image ISO pour l'instant.")).size(13).color(theme::SECONDARY));
                }
                for (name, size) in images {
                    list = list.push(
                        row![
                            text(format_size(*size, lang)).size(13).width(80),
                            text(name.as_str()).size(13).font(iced::Font::MONOSPACE),
                        ]
                        .spacing(8),
                    );
                }
                column![
                    list,
                    row![
                        action_button(lang.pick("Add an ISO...", "Ajouter une ISO..."), Some(VaultMessage::Browse), true),
                        action_button(lang.pick("Update GRUB", "Mettre à jour GRUB"), Some(VaultMessage::Update), false),
                        close,
                    ]
                    .spacing(8),
                ]
                .spacing(8)
                .into()
            }
            VaultState::Finished { outcome, .. } => column![
                text(phrases::outcome_with_explain(lang, outcome)).size(13),
                action_button(lang.pick("OK", "OK"), Some(VaultMessage::Open), true),
            ]
            .spacing(8)
            .into(),
        };
        Some(super::boxed(column![title, body].spacing(8).into()))
    }
}
