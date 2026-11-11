use std::time::Instant;

use bonk_hal::Disk;
use iced::Element;
use iced::widget::{button, checkbox, column, progress_bar, row, text, text_input};

use bonk_core::write::Phase;

use crate::theme;
use crate::transfer::Outcome;
use bonk_ui::text as phrases;

use super::super::catalog::{CatalogState, find_url, menu_button, menu_list};
use super::super::{App, Focus, Message, VaultMessage, WRITE_DELAY, WriteState};
use super::{boxed, ring};

impl App {
    /// Summary, image input, a button disabled for 3 seconds, progress, cancel, result.
    pub(super) fn write_panel<'a>(&'a self, disk: &'a Disk) -> Option<Element<'a, Message>> {
        let WriteState::Draft {
            disk_id,
            image,
            hash,
            raw,
            mbr,
            opened,
        } = &self.write
        else {
            return None;
        };
        // One erasing action on screen at a time: the vault panel replaces the write panel of its disk.
        (disk_id == &disk.id && self.vault.disk_id() != Some(disk.id.as_str()))
            .then(|| boxed(self.draft_panel(disk, image, hash, (*raw, *mbr), *opened)))
    }

    /// Above the list, independent of it: the disk may be unplugged while the child still runs, and it must
    /// stay cancellable.
    pub(super) fn active_panel(&self) -> Option<Element<'_, Message>> {
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

    /// The button only enables after the delay to think.
    fn draft_panel<'a>(
        &'a self,
        disk: &'a Disk,
        image: &'a str,
        hash: &'a str,
        (raw, mbr): (bool, bool),
        opened: Instant,
    ) -> Element<'a, Message> {
        let lang = self.lang;
        // One clock read per render: the countdown, the button state and its label cannot disagree.
        let elapsed = opened.elapsed();
        let ready = elapsed >= WRITE_DELAY && !image.trim().is_empty();
        let label = phrases::write_button(lang, elapsed, image.trim().is_empty());
        let mut panel = column![
            text(lang.pick(
                "WRITE AN IMAGE TO THIS DISK",
                "ÉCRIRE UNE IMAGE SUR CE DISQUE"
            ))
            .size(14),
            text(phrases::disk_summary(lang, disk)).size(13),
            text(lang.pick(
                "All the data on this disk will be erased. This cannot be undone.",
                "Toutes les données de ce disque seront effacées. Cette action est irréversible.",
            ))
            .size(13)
            .color(theme::SECONDARY),
            ring(
                menu_button(&self.catalog, image, self.menu_open, lang),
                self.has_focus(Focus::Menu),
            ),
        ]
        .spacing(8);
        if let (CatalogState::Ready(systems), true) = (&self.catalog, self.menu_open) {
            panel = panel.push(menu_list(systems, &self.menu_filter, self.menu_index, lang));
        }
        if let CatalogState::Failed(error) = &self.catalog {
            panel = panel.push(text(error.clone()).size(13).color(theme::SECONDARY));
        }
        if let CatalogState::Ready(systems) = &self.catalog
            && find_url(systems, image).is_some()
        {
            panel = panel.push(
                text(lang.pick(
                    "Official link and hash from the signed catalog: nothing to set.",
                    "Lien officiel et empreinte du catalogue signé : rien à régler.",
                ))
                .size(13)
                .color(theme::SECONDARY),
            );
        }
        panel = panel.push(ring(
            button(
                text(if self.advanced {
                    lang.pick(
                        "Hide the manual image and options",
                        "Masquer l'image manuelle et les options",
                    )
                } else {
                    lang.pick("Other image or options...", "Autre image ou options...")
                })
                .size(13),
            )
            .on_press(Message::ToggleAdvanced)
            .padding([4, 10])
            .style(theme::card_button(false)),
            self.has_focus(Focus::Advanced),
        ));
        if self.advanced {
            panel = panel.push(self.manual_options(image, hash, raw, mbr));
        }
        // Several ISO images on one drive instead of one image: the vault reads the drive before offering
        // anything.
        if !self.vault.disk_id().is_some_and(|id| id == disk.id) {
            panel = panel.push(
                button(text(bonk_ui::vault::VaultWords::new(lang).open).size(13))
                    .on_press(Message::Vault(VaultMessage::Open))
                    .padding([4, 10])
                    .style(theme::card_button(false)),
            );
        }
        panel
            .push(ring(
                button(text(label).size(14))
                    .on_press_maybe(ready.then_some(Message::StartWrite))
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::Action),
            ))
            .into()
    }

    fn manual_options<'a>(
        &'a self,
        image: &'a str,
        hash: &'a str,
        raw: bool,
        mbr: bool,
    ) -> Element<'a, Message> {
        let lang = self.lang;
        column![
            row![
                ring(
                    text_input(
                        lang.pick(
                            "Path or https:// address of the image (.img, .iso, .gz, .xz, .zst)",
                            "Chemin ou adresse https:// de l'image (.img, .iso, .gz, .xz, .zst)",
                        ),
                        image
                    )
                    .on_input(Message::ImageInput)
                    .on_submit(Message::StartWrite)
                    .padding(8)
                    .size(14)
                    .style(theme::input_style),
                    self.has_focus(Focus::Image),
                ),
                ring(
                    button(text(lang.pick("Choose the image...", "Choisir l'image...")).size(14))
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
                    lang.pick(
                        "SHA-256 hash, or path or address of SHA256SUMS (required for an address)",
                        "Empreinte SHA-256, ou chemin ou adresse de SHA256SUMS (exigée pour une adresse)",
                    ),
                    hash
                )
                .on_input(Message::HashInput)
                .padding(8)
                .size(14)
                .style(theme::input_style),
                self.has_focus(Focus::Hash),
            ),
            ring(
                checkbox(self.use_tor)
                    .label(match &self.tor {
                        Some(_) => lang.pick(
                            "Download through Tor: slower, without showing your address to the publisher",
                            "Télécharger par Tor : plus lent, sans montrer votre adresse à l'éditeur",
                        ),
                        None => lang.pick(
                            "Download through Tor: no Tor detected on this computer",
                            "Télécharger par Tor : aucun Tor détecté sur cet ordinateur",
                        ),
                    })
                    .on_toggle_maybe(self.tor.is_some().then_some(Message::ToggleTor))
                    .size(16)
                    .text_size(13),
                self.has_focus(Focus::Tor),
            ),
            ring(
                checkbox(raw)
                    .label(lang.pick(
                        "Write the bytes as they are, without refusing a non-bootable image",
                        "Écrire les octets tels quels, sans refuser une image non amorçable",
                    ))
                    .on_toggle(Message::ToggleRaw)
                    .size(16)
                    .text_size(13),
                self.has_focus(Focus::Raw),
            ),
            ring(
                checkbox(mbr)
                    .label(lang.pick(
                        "Windows drive: MBR table, boots in BIOS and UEFI up to 2 TiB (unchecked: GPT, UEFI only)",
                        "Clé Windows : table MBR, démarre en BIOS et en UEFI jusqu'à 2 Tio (décochée : GPT, UEFI seul)",
                    ))
                    .on_toggle(Message::ToggleMbr)
                    .size(16)
                    .text_size(13),
                self.has_focus(Focus::Mbr),
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
        let lang = self.lang;
        let (line, fraction) = phrases::progress(lang, phase, done, total, rate);
        column![
            text(line).size(14),
            progress_bar(0.0..=1.0, fraction).girth(14),
            ring(
                button(text(lang.pick("Cancel", "Annuler")).size(14))
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
        let lang = self.lang;
        let message = phrases::outcome_with_explain(lang, outcome);
        column![
            text(message).size(14),
            ring(
                button(text(lang.pick("Close", "Fermer")).size(14))
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
