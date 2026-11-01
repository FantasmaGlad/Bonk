//! Rendu de la fenêtre : liste, cartes, détail, panneau d'écriture et demande de confirmation.

use std::time::{Duration, Instant};

use bonk_hal::{
    Disk, Eligibility, Lang, bus_label, format_size, kind_label, state_label, verdict_badge,
};
use iced::widget::{
    Column, button, center, checkbox, column, container, opaque, progress_bar, row, scrollable,
    stack, text, text_input,
};
use iced::{Element, Fill};

use bonk_core::write::Phase;

use crate::errors::failure_text;
use crate::theme;
use crate::transfer::Outcome;

use super::catalog::{CatalogState, find_url, menu_button, menu_list};
use super::navigation::LIST;
use super::{App, Focus, Message, REFRESH_EVERY, WRITE_DELAY, WriteState};

/// Demande de confirmation avant d'effacer un disque interne.
pub(super) fn internal_dialog(disk: &Disk, lang: Lang) -> Element<'_, Message> {
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
pub(super) fn detail(disk: &Disk, lang: Lang) -> Element<'_, Message> {
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

fn boxed<'a>(body: Element<'a, Message>) -> Element<'a, Message> {
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

    /// Panneau d'écriture du disque sélectionné : récapitulatif, saisie de l'image, bouton
    /// désactivé pendant 3 secondes, progression, annulation, résultat.
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
        (disk_id == &disk.id)
            .then(|| boxed(self.draft_panel(disk, image, hash, (*raw, *mbr), *opened)))
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

    fn has_focus(&self, target: Focus) -> bool {
        self.focus.as_ref() == Some(&target)
    }

    /// Récapitulatif, saisie de l'image et bouton qui ne s'active qu'après le délai de réflexion.
    fn draft_panel<'a>(
        &'a self,
        disk: &'a Disk,
        image: &'a str,
        hash: &'a str,
        (raw, mbr): (bool, bool),
        opened: Instant,
    ) -> Element<'a, Message> {
        let lang = self.lang;
        // Une seule lecture de l'horloge par affichage : le décompte, l'état du bouton et son libellé
        // ne peuvent pas se contredire.
        let elapsed = opened.elapsed();
        let ready = elapsed >= WRITE_DELAY && !image.trim().is_empty();
        let label = if elapsed < WRITE_DELAY {
            let remaining = secondes_restantes(elapsed);
            match lang {
                Lang::En => format!("Write and erase this disk (in {remaining} s)"),
                Lang::Fr => format!("Écrire et effacer ce disque (dans {remaining} s)"),
            }
        } else if image.trim().is_empty() {
            lang.pick(
                "Write and erase this disk (choose an image)",
                "Écrire et effacer ce disque (choisir une image)",
            )
            .to_owned()
        } else {
            lang.pick("Write and erase this disk", "Écrire et effacer ce disque")
                .to_owned()
        };
        let labels: Vec<&str> = disk
            .partitions
            .iter()
            .filter_map(|p| p.label.as_deref())
            .collect();
        let mut panel = column![
            text(lang.pick(
                "WRITE AN IMAGE TO THIS DISK",
                "ÉCRIRE UNE IMAGE SUR CE DISQUE"
            ))
            .size(14),
            text(format!(
                "{}  {} {}  {}  {} partition(s){}",
                disk.model,
                lang.pick("serial", "série"),
                disk.serial
                    .as_deref()
                    .unwrap_or(lang.pick("unknown", "inconnue")),
                format_size(disk.size_bytes, lang),
                disk.partitions.len(),
                if labels.is_empty() {
                    String::new()
                } else {
                    format!(
                        "  {} {}",
                        lang.pick("labels:", "étiquettes :"),
                        labels.join(", ")
                    )
                },
            ))
            .size(13),
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
            panel = panel.push(
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
                                "Windows drive (extraction, FAT32, UEFI boot): MBR table instead of GPT",
                                "Clé Windows (extraction, FAT32, démarrage UEFI) : table MBR au lieu de GPT",
                            ))
                            .on_toggle(Message::ToggleMbr)
                            .size(16)
                            .text_size(13),
                        self.has_focus(Focus::Mbr),
                    ),
                ]
                .spacing(8),
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

    fn running_panel(
        &self,
        phase: Phase,
        done: u64,
        total: Option<u64>,
        rate: f64,
    ) -> Element<'_, Message> {
        let lang = self.lang;
        let step = match phase {
            Phase::Download => lang.pick("Download", "Téléchargement"),
            Phase::Source => lang.pick("Checking the image", "Contrôle de l'image"),
            Phase::Authorize => lang.pick(
                "Waiting for the system's authorization",
                "En attente de l'autorisation du système",
            ),
            Phase::Write => lang.pick("Writing", "Écriture"),
            Phase::Flush => lang.pick("Flushing buffers", "Vidage des tampons"),
            Phase::Verify => lang.pick("Verifying", "Vérification"),
        };
        let fraction = total
            .filter(|t| *t > 0)
            .map_or(0.0, |t| (done as f32 / t as f32).clamp(0.0, 1.0));
        column![
            text(format!(
                "{step}{} {}{}  ({}/s)",
                lang.pick(":", " :"),
                format_size(done, lang),
                total.map_or_else(String::new, |t| format!(
                    " {} {}",
                    lang.pick("of", "sur"),
                    format_size(t, lang)
                )),
                format_size(rate as u64, lang),
            ))
            .size(14),
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
        let message = match outcome {
            Outcome::Done {
                bytes,
                verified,
                gpt_repaired,
                secure_boot,
            } => format!(
                "{} {} {}{}{}.{}",
                lang.pick("WRITE FINISHED:", "ÉCRITURE TERMINÉE :"),
                format_size(*bytes, lang),
                lang.pick("written", "écrits"),
                if *verified {
                    lang.pick(", read back and verified", ", relus et vérifiés")
                } else {
                    ""
                },
                if *gpt_repaired {
                    lang.pick(", backup GPT repaired", ", GPT de secours réparé")
                } else {
                    ""
                },
                secure_boot
                    .as_ref()
                    .map_or_else(String::new, |text| format!(" Secure Boot : {text}.")),
            ),
            Outcome::Failed { code, message } => format!(
                "{} {}{}",
                lang.pick("FAILED:", "ÉCHEC :"),
                failure_text(lang, code.as_deref(), message),
                code.as_ref().map_or_else(String::new, |c| match lang {
                    Lang::En => format!(" [{c}]: see \"bonk explain {c}\""),
                    Lang::Fr => format!(" [{c}] : voir « bonk explain {c} »"),
                }),
            ),
        };
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
