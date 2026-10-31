//! Rendu de la fenêtre : liste, cartes, détail, panneau d'écriture et demande de confirmation.

use std::time::{Duration, Instant};

use bonk_hal::{Disk, Eligibility, bus_label, format_size, kind_label, state_label, verdict_badge};
use iced::widget::{
    Column, button, center, checkbox, column, container, opaque, progress_bar, row, scrollable,
    stack, text, text_input,
};
use iced::{Element, Fill};

use bonk_core::write::Phase;

use crate::theme;
use crate::transfer::Outcome;

use super::navigation::LIST;
use super::{App, Focus, Message, REFRESH_EVERY, WRITE_DELAY, WriteState};

/// Demande de confirmation avant d'effacer un disque interne.
pub(super) fn internal_dialog(disk: &Disk) -> Element<'_, Message> {
    container(
        column![
            text("ATTENTION : DISQUE INTERNE").size(18),
            text(format!(
                "{} ({}, {}) fait partie de cette machine. Toutes ses données seront effacées, sans \
                 retour possible.",
                disk.path,
                disk.model,
                format_size(disk.size_bytes)
            ))
            .size(14),
            row![
                button(text("Renoncer").size(14))
                    .on_press(Message::CloseInternal)
                    .padding([6, 14])
                    .style(theme::primary_button),
                button(text("Effacer ce disque interne").size(14))
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

/// Secondes affichées dans le décompte du délai de réflexion, selon le temps écoulé depuis l'ouverture.
pub(super) fn secondes_restantes(elapsed: Duration) -> u64 {
    let reste = WRITE_DELAY.saturating_sub(elapsed);
    reste.as_secs() + u64::from(reste.subsec_nanos() > 0)
}

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

/// Récapitulatif d'un disque : série, nature, partitions, étiquettes, montages.
pub(super) fn detail(disk: &Disk) -> Element<'_, Message> {
    let mut lines = column![
        text(format!(
            "Série : {}   Nature : {}   Drapeau amovible du système : {}",
            disk.serial.as_deref().unwrap_or("inconnue"),
            kind_label(disk.kind),
            if disk.removable_flag { "oui" } else { "non" },
        ))
        .size(13)
        .color(theme::SECONDARY),
    ]
    .spacing(2);

    if disk.partitions.is_empty() {
        lines = lines.push(
            text("Aucune partition détectée.")
                .size(13)
                .color(theme::SECONDARY),
        );
    }
    for partition in &disk.partitions {
        let mounts = if partition.mount_points.is_empty() {
            "non monté".to_owned()
        } else {
            format!("monté sur {}", partition.mount_points.join(", "))
        };
        lines = lines.push(
            text(format!(
                "{}  {}  {}  étiquette : {}  {}",
                partition.path,
                format_size(partition.size_bytes),
                partition
                    .fs_type
                    .as_deref()
                    .unwrap_or("système de fichiers inconnu"),
                partition.label.as_deref().unwrap_or("aucune"),
                mounts,
            ))
            .size(13)
            .font(iced::Font::MONOSPACE),
        );
    }
    container(lines).padding([8, 0]).into()
}

fn boxed<'a>(body: Element<'a, Message>) -> Element<'a, Message> {
    container(body)
        .padding(12)
        .width(Fill)
        .style(theme::panel)
        .into()
}

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        let entries = self.entries();
        let shown = entries.len();

        let header = column![
            text("BONK").size(28),
            text("Détection des disques. Une écriture exige un disque proposé, un récapitulatif et trois secondes de réflexion.")
                .size(14)
                .color(theme::SECONDARY),
        ]
        .spacing(4);

        let controls = self.controls();

        let body = match self.active_panel() {
            Some(panel) => column![panel, self.disk_list(entries)].spacing(8).into(),
            None => self.disk_list(entries),
        };

        let equivalent = row![
            column![
                text("Commande équivalente")
                    .size(13)
                    .color(theme::SECONDARY),
                text(self.equivalents().join("\n"))
                    .size(13)
                    .font(iced::Font::MONOSPACE),
            ]
            .width(Fill),
            ring(
                button(text("Copier").size(14))
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
            Some(disk) => stack![window, opaque(center(internal_dialog(disk)))].into(),
            None => window.into(),
        }
    }

    pub(super) fn status_line(&self, shown: usize) -> String {
        if self.error.is_some() {
            return format!(
                "Détection en échec. Nouvelle tentative automatique toutes les {} s.",
                REFRESH_EVERY.as_secs()
            );
        }
        if self.loading && self.disks.is_empty() {
            return "Détection en cours...".to_owned();
        }
        format!(
            "{shown} disque(s) affiché(s) sur {} détecté(s). Actualisation automatique toutes les {} s.",
            self.disks.len(),
            REFRESH_EVERY.as_secs()
        )
    }

    pub(super) fn card<'a>(&'a self, disk: &'a Disk, verdict: Eligibility) -> Element<'a, Message> {
        let selected = self.selected.as_deref() == Some(disk.id.as_str());

        let top = row![
            text(disk.model.as_str()).size(16).width(Fill),
            text(verdict_badge(&verdict)).size(14),
        ];
        let facts = row![
            text(disk.path.as_str())
                .size(13)
                .font(iced::Font::MONOSPACE),
            text(format_size(disk.size_bytes)).size(13),
            text(bus_label(disk.bus)).size(13),
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
            content = content.push(text(state_label(&verdict)).size(13).color(theme::SECONDARY));
        }
        if selected {
            content = content.push(detail(disk));
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

    /// Panneau d'écriture du disque sélectionné : récapitulatif, saisie de l'image, bouton
    /// désactivé pendant 3 secondes, progression, annulation, résultat.
    pub(super) fn write_panel<'a>(&'a self, disk: &'a Disk) -> Option<Element<'a, Message>> {
        let WriteState::Draft {
            disk_id,
            image,
            hash,
            raw,
            opened,
        } = &self.write
        else {
            return None;
        };
        (disk_id == &disk.id).then(|| boxed(self.draft_panel(disk, image, hash, *raw, *opened)))
    }

    /// Écriture en cours ou résultat, au-dessus de la liste : ils ne dépendent pas d'elle. Le disque a pu
    /// être débranché, le sous-processus tourne encore, et l'on doit pouvoir l'annuler.
    fn active_panel(&self) -> Option<Element<'_, Message>> {
        match &self.write {
            WriteState::Running {
                phase,
                done,
                total,
                rate,
                ..
            } => Some(boxed(self.running_panel(*phase, *done, *total, *rate))),
            WriteState::Finished { outcome, .. } => Some(boxed(self.finished_panel(outcome))),
            WriteState::Idle | WriteState::Draft { .. } => None,
        }
    }

    /// Actualisation et réglages d'affichage.
    fn controls(&self) -> Element<'_, Message> {
        row![
            ring(
                button(text("Actualiser").size(14))
                    .on_press(Message::Refresh)
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::Refresh),
            ),
            ring(
                checkbox(self.show_internal)
                    .label("Disques internes non système")
                    .on_toggle(Message::ToggleInternal)
                    .size(18)
                    .style(theme::checkbox_style),
                self.has_focus(Focus::Internal),
            ),
            ring(
                checkbox(self.allow_virtual)
                    .label("Disques virtuels (mode test)")
                    .on_toggle(Message::ToggleVirtual)
                    .size(18)
                    .style(theme::checkbox_style),
                self.has_focus(Focus::Virtual),
            ),
            ring(
                checkbox(self.show_all)
                    .label("Afficher aussi les disques écartés")
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
        if let Some(erreur) = &self.error {
            container(
                column![
                    text("ERREUR : les disques n'ont pas pu être détectés.").size(16),
                    text(erreur.as_str()).size(14).color(theme::SECONDARY),
                    text("Aucun disque n'est proposé tant que la détection échoue.")
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
                "Détection en cours...".to_owned()
            } else if self.show_all {
                "Aucun disque détecté.".to_owned()
            } else {
                "Aucun disque proposé. Branchez une clé USB ou une carte SD, ou cochez « Afficher aussi \
                 les disques écartés » pour voir pourquoi les disques de cette machine sont écartés."
                    .to_owned()
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

    fn has_focus(&self, target: Focus) -> bool {
        self.focus.as_ref() == Some(&target)
    }

    /// Récapitulatif, saisie de l'image et bouton qui ne s'active qu'après le délai de réflexion.
    fn draft_panel<'a>(
        &'a self,
        disk: &'a Disk,
        image: &'a str,
        hash: &'a str,
        raw: bool,
        opened: Instant,
    ) -> Element<'a, Message> {
        // Une seule lecture de l'horloge par affichage : le décompte, l'état du bouton et son libellé
        // ne peuvent pas se contredire.
        let elapsed = opened.elapsed();
        let ready = elapsed >= WRITE_DELAY && !image.trim().is_empty();
        let label = if elapsed < WRITE_DELAY {
            let remaining = secondes_restantes(elapsed);
            format!("Écrire et effacer ce disque (dans {remaining} s)")
        } else if image.trim().is_empty() {
            "Écrire et effacer ce disque (saisir une image)".to_owned()
        } else {
            "Écrire et effacer ce disque".to_owned()
        };
        let labels: Vec<&str> = disk
            .partitions
            .iter()
            .filter_map(|p| p.label.as_deref())
            .collect();
        column![
            text("ÉCRIRE UNE IMAGE SUR CE DISQUE").size(14),
            text(format!(
                "{}  série {}  {}  {} partition(s){}",
                disk.model,
                disk.serial.as_deref().unwrap_or("inconnue"),
                format_size(disk.size_bytes),
                disk.partitions.len(),
                if labels.is_empty() {
                    String::new()
                } else {
                    format!("  étiquettes : {}", labels.join(", "))
                },
            ))
            .size(13),
            text("Toutes les données de ce disque seront effacées. Cette action est irréversible.")
                .size(13)
                .color(theme::SECONDARY),
            row![
                ring(
                    text_input("Chemin ou adresse https:// de l'image (.img, .iso, .gz, .xz, .zst)", image)
                        .on_input(Message::ImageInput)
                        .on_submit(Message::StartWrite)
                        .padding(8)
                        .size(14)
                        .style(theme::input_style),
                    self.has_focus(Focus::Image),
                ),
                ring(
                    button(text("Choisir l'image...").size(14))
                        .on_press(Message::BrowseImage)
                        .padding([6, 14])
                        .style(theme::primary_button),
                    self.has_focus(Focus::Browse),
                ),
            ]
            .spacing(8)
            .align_y(iced::Center),
            ring(
                text_input(
                    "Empreinte SHA-256, ou chemin ou adresse de SHA256SUMS (exigée pour une adresse)",
                    hash
                )
                .on_input(Message::HashInput)
                .padding(8)
                .size(14)
                .style(theme::input_style),
                self.has_focus(Focus::Hash),
            ),
            ring(
                checkbox(raw)
                    .label("Écrire les octets tels quels, sans refuser une image non amorçable")
                    .on_toggle(Message::ToggleRaw)
                    .size(16)
                    .text_size(13),
                self.has_focus(Focus::Raw),
            ),
            ring(
                button(text(label).size(14))
                    .on_press_maybe(ready.then_some(Message::StartWrite))
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::Action),
            ),
        ]
        .spacing(8)
        .into()
    }

    fn running_panel(
        &self,
        phase: Phase,
        done: u64,
        total: Option<u64>,
        rate: f64,
    ) -> Element<'_, Message> {
        let step = match phase {
            Phase::Download => "Téléchargement",
            Phase::Source => "Contrôle de l'image",
            Phase::Authorize => "En attente de l'autorisation du système",
            Phase::Write => "Écriture",
            Phase::Flush => "Vidage des tampons",
            Phase::Verify => "Vérification",
        };
        let fraction = total
            .filter(|t| *t > 0)
            .map_or(0.0, |t| (done as f32 / t as f32).clamp(0.0, 1.0));
        column![
            text(format!(
                "{step} : {}{}  ({}/s)",
                format_size(done),
                total.map_or_else(String::new, |t| format!(" sur {}", format_size(t))),
                format_size(rate as u64),
            ))
            .size(14),
            progress_bar(0.0..=1.0, fraction).girth(14),
            ring(
                button(text("Annuler").size(14))
                    .on_press(Message::CancelWrite)
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::Action),
            ),
        ]
        .spacing(8)
        .into()
    }

    fn finished_panel<'a>(&'a self, outcome: &Outcome) -> Element<'a, Message> {
        let message = match outcome {
            Outcome::Done {
                bytes,
                verified,
                gpt_repaired,
            } => format!(
                "ÉCRITURE TERMINÉE : {} écrits{}{}.",
                format_size(*bytes),
                if *verified {
                    ", relus et vérifiés"
                } else {
                    ""
                },
                if *gpt_repaired {
                    ", GPT de secours réparé"
                } else {
                    ""
                },
            ),
            Outcome::Failed { code, message } => format!(
                "ÉCHEC : {message}{}",
                code.as_ref().map_or_else(String::new, |c| format!(
                    " [{c}] : voir « bonk explain {c} »"
                )),
            ),
        };
        column![
            text(message).size(14),
            ring(
                button(text("Fermer").size(14))
                    .on_press(Message::DismissWrite)
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::Action),
            ),
        ]
        .spacing(8)
        .into()
    }
}
