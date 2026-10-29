//! Fenêtre de détection des disques : liste, verdict du filtre de sécurité, détail d'un disque.
//!
//! Elle est en lecture seule : aucun bouton d'écriture n'existe. Comme `bonk list`, elle ne fait que
//! présenter le verdict de `bonk_hal::evaluate`, sans aucune règle propre.

use std::time::Duration;

use bonk_hal::{
    Disk, Eligibility, FilterOptions, bus_label, enumerate_disks, evaluate, format_size,
    kind_label, state_label, verdict_badge,
};
#[cfg(test)]
use iced::Size;
use iced::widget::{Column, button, checkbox, column, container, row, scrollable, text};
use iced::{Element, Fill, Subscription, Task, Theme};

use crate::theme;

/// Intervalle d'actualisation automatique : un disque branché apparaît sans geste de l'utilisateur.
const REFRESH_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Refresh,
    Tick,
    /// Résultat d'une énumération ; l'erreur est déjà mise en texte (`EnumerateError` n'est pas `Clone`).
    Loaded(Result<Vec<Disk>, String>),
    ToggleInternal(bool),
    ToggleVirtual(bool),
    ToggleAll(bool),
    /// Sélectionne (ou désélectionne) un disque, par identifiant.
    Select(String),
}

#[derive(Debug, Default)]
pub(crate) struct App {
    disks: Vec<Disk>,
    loading: bool,
    error: Option<String>,
    show_internal: bool,
    allow_virtual: bool,
    show_all: bool,
    selected: Option<String>,
}

fn load() -> Task<Message> {
    Task::perform(
        async { enumerate_disks().map_err(|erreur| erreur.to_string()) },
        Message::Loaded,
    )
}

impl App {
    pub(crate) fn new() -> (Self, Task<Message>) {
        (
            Self {
                loading: true,
                ..Self::default()
            },
            load(),
        )
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // Une seule énumération à la fois : le minuteur ne les empile pas si l'une est lente.
            Message::Refresh | Message::Tick => {
                if self.loading {
                    Task::none()
                } else {
                    self.loading = true;
                    load()
                }
            }
            Message::Loaded(Ok(disks)) => {
                self.loading = false;
                self.error = None;
                // Un disque débranché ne reste pas sélectionné.
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|id| !disks.iter().any(|d| &d.id == id))
                {
                    self.selected = None;
                }
                self.disks = disks;
                Task::none()
            }
            Message::Loaded(Err(erreur)) => {
                self.loading = false;
                // Jamais de liste périmée présentée comme valable après un échec.
                self.disks.clear();
                self.selected = None;
                self.error = Some(erreur);
                Task::none()
            }
            Message::ToggleInternal(on) => {
                self.show_internal = on;
                Task::none()
            }
            Message::ToggleVirtual(on) => {
                self.allow_virtual = on;
                Task::none()
            }
            Message::ToggleAll(on) => {
                self.show_all = on;
                Task::none()
            }
            Message::Select(id) => {
                self.selected = if self.selected.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id)
                };
                Task::none()
            }
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        iced::time::every(REFRESH_EVERY).map(|_| Message::Tick)
    }

    pub(crate) fn theme(&self) -> Theme {
        theme::bonk()
    }

    /// Disques à afficher avec leur verdict, selon les réglages.
    fn entries(&self) -> Vec<(&Disk, Eligibility)> {
        let options = FilterOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
        };
        self.disks
            .iter()
            .map(|disk| (disk, evaluate(disk, options)))
            .filter(|(_, verdict)| self.show_all || verdict.is_writable())
            .collect()
    }

    fn status_line(&self, shown: usize) -> String {
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

    pub(crate) fn view(&self) -> Element<'_, Message> {
        let entries = self.entries();
        let shown = entries.len();

        let header = column![
            text("BONK").size(28),
            text("Détection des disques. Lecture seule : aucune écriture n'est possible depuis cette fenêtre.")
                .size(14)
                .color(theme::SECONDARY),
        ]
        .spacing(4);

        let controls = row![
            button(text("Actualiser").size(14))
                .on_press(Message::Refresh)
                .padding([6, 14])
                .style(theme::primary_button),
            checkbox(self.show_internal)
                .label("Disques internes non système")
                .on_toggle(Message::ToggleInternal)
                .size(18)
                .style(theme::checkbox_style),
            checkbox(self.allow_virtual)
                .label("Disques virtuels (mode test)")
                .on_toggle(Message::ToggleVirtual)
                .size(18)
                .style(theme::checkbox_style),
            checkbox(self.show_all)
                .label("Afficher aussi les disques écartés")
                .on_toggle(Message::ToggleAll)
                .size(18)
                .style(theme::checkbox_style),
        ]
        .spacing(20)
        .align_y(iced::Center);

        let body: Element<'_, Message> = if let Some(erreur) = &self.error {
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
            let cards: Vec<Element<'_, Message>> = entries
                .into_iter()
                .map(|(disk, verdict)| self.card(disk, verdict))
                .collect();
            scrollable(Column::with_children(cards).spacing(8).width(Fill))
                .height(Fill)
                .into()
        };

        container(
            column![
                header,
                controls,
                text(self.status_line(shown))
                    .size(13)
                    .color(theme::SECONDARY),
                body
            ]
            .spacing(14),
        )
        .padding(20)
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn card<'a>(&'a self, disk: &'a Disk, verdict: Eligibility) -> Element<'a, Message> {
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

        button(content)
            .on_press(Message::Select(disk.id.clone()))
            .padding(12)
            .width(Fill)
            .style(theme::card_button(selected))
            .into()
    }
}

/// Récapitulatif d'un disque (CDC §3.3) : série, nature, partitions, étiquettes, montages.
fn detail(disk: &Disk) -> Element<'_, Message> {
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

#[cfg(test)]
mod tests {
    use bonk_hal::{Bus, DiskKind, Partition};
    use iced_test::Simulator;

    use super::*;

    fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
        Partition {
            path: path.to_owned(),
            label: Some("CLE".to_owned()),
            fs_type: Some("vfat".to_owned()),
            size_bytes: 15_000_000_000,
            mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
            holders: vec![],
            is_system_volume: system,
            is_active_esp: false,
        }
    }

    fn disk(id: &str, path: &str, model: &str, bus: Bus, partitions: Vec<Partition>) -> Disk {
        Disk {
            id: id.to_owned(),
            path: path.to_owned(),
            model: model.to_owned(),
            serial: Some("4C530001".to_owned()),
            size_bytes: 16_008_609_792,
            bus,
            kind: DiskKind::Physical,
            removable_flag: true,
            fs_type: None,
            holders: vec![],
            partitions,
        }
    }

    fn sample() -> Vec<Disk> {
        vec![
            disk(
                "nvme-systeme",
                "/dev/nvme0n1",
                "Disque systeme",
                Bus::Nvme,
                vec![partition("/dev/nvme0n1p2", &["/"], true)],
            ),
            disk(
                "usb-cle",
                "/dev/sdb",
                "Cle de test",
                Bus::Usb,
                vec![partition("/dev/sdb1", &["/run/media/alice/CLE"], false)],
            ),
            disk(
                "sata-libre",
                "/dev/sda",
                "Disque interne libre",
                Bus::Sata,
                vec![],
            ),
        ]
    }

    fn loaded(disks: Vec<Disk>) -> App {
        let mut app = App::default();
        let _ = app.update(Message::Loaded(Ok(disks)));
        app
    }

    fn visible_ids(app: &App) -> Vec<&str> {
        app.entries().iter().map(|(d, _)| d.id.as_str()).collect()
    }

    #[test]
    fn par_defaut_seuls_les_disques_proposes_sont_affiches() {
        assert_eq!(visible_ids(&loaded(sample())), ["usb-cle"]);
    }

    #[test]
    fn les_cases_changent_l_affichage_sans_nouvelle_enumeration() {
        let mut app = loaded(sample());
        let _ = app.update(Message::ToggleAll(true));
        assert_eq!(visible_ids(&app), ["nvme-systeme", "usb-cle", "sata-libre"]);
        let _ = app.update(Message::ToggleAll(false));
        let _ = app.update(Message::ToggleInternal(true));
        assert_eq!(visible_ids(&app), ["usb-cle", "sata-libre"]);
        assert!(!app.loading);
    }

    #[test]
    fn un_disque_systeme_n_est_jamais_affiche_comme_propose() {
        let mut app = loaded(sample());
        for message in [
            Message::ToggleAll(true),
            Message::ToggleInternal(true),
            Message::ToggleVirtual(true),
        ] {
            let _ = app.update(message);
        }
        let entries = app.entries();
        let system = entries.iter().find(|(d, _)| d.id == "nvme-systeme");
        assert!(system.is_some_and(|(_, verdict)| !verdict.is_writable()));
    }

    #[test]
    fn la_selection_bascule_et_disparait_avec_le_disque() {
        let mut app = loaded(sample());
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert_eq!(app.selected.as_deref(), Some("usb-cle"));
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert_eq!(app.selected, None);
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        // La clé est débranchée : la sélection tombe.
        let _ = app.update(Message::Loaded(Ok(vec![])));
        assert_eq!(app.selected, None);
    }

    #[test]
    fn une_enumeration_a_la_fois() {
        let mut app = App::default();
        let _ = app.update(Message::Refresh);
        assert!(app.loading);
        // Le minuteur ne relance rien tant que la précédente n'est pas finie : l'état reste « en cours ».
        let _ = app.update(Message::Tick);
        assert!(app.loading);
        let _ = app.update(Message::Loaded(Ok(vec![])));
        assert!(!app.loading);
    }

    #[test]
    fn un_echec_vide_la_liste_au_lieu_de_la_garder() {
        let mut app = loaded(sample());
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        let _ = app.update(Message::Loaded(Err("base udev introuvable".to_owned())));
        assert!(app.disks.is_empty());
        assert_eq!(app.selected, None);
        assert_eq!(app.error.as_deref(), Some("base udev introuvable"));
        // Une énumération réussie efface l'erreur.
        let _ = app.update(Message::Loaded(Ok(sample())));
        assert_eq!(app.error, None);
    }

    fn contient(app: &App, texte: &str) -> bool {
        Simulator::new(app.view()).find(texte).is_ok()
    }

    #[test]
    fn la_fenetre_affiche_la_cle_et_pas_le_disque_systeme() {
        let app = loaded(sample());
        assert!(contient(&app, "Cle de test"));
        assert!(contient(&app, "PROPOSÉ"));
        assert!(!contient(&app, "Disque systeme"));
        // Pas de bouton d'écriture : la fenêtre est en lecture seule.
        assert!(!contient(&app, "Écrire"));
    }

    #[test]
    fn la_fenetre_explique_pourquoi_un_disque_est_ecarte() {
        let mut app = loaded(sample());
        let _ = app.update(Message::ToggleAll(true));
        assert!(contient(&app, "INTERDIT"));
        assert!(contient(
            &app,
            "interdit : volume système (/dev/nvme0n1p2) ; /dev/nvme0n1p2 monté sur /"
        ));
    }

    #[test]
    fn un_clic_sur_la_carte_ouvre_le_detail() -> Result<(), Box<dyn std::error::Error>> {
        let mut app = loaded(sample());
        let mut simulateur = Simulator::new(app.view());
        simulateur.click("Cle de test")?;
        let messages: Vec<Message> = simulateur.into_messages().collect();
        assert!(
            matches!(messages.as_slice(), [Message::Select(id)] if id == "usb-cle"),
            "{messages:?}"
        );
        for message in messages {
            let _ = app.update(message);
        }
        assert!(contient(
            &app,
            "Série : 4C530001   Nature : physique   Drapeau amovible du système : oui"
        ));
        Ok(())
    }

    #[test]
    fn un_echec_s_affiche_en_toutes_lettres() {
        let mut app = App::default();
        let _ = app.update(Message::Loaded(Err("base udev introuvable".to_owned())));
        assert!(contient(
            &app,
            "ERREUR : les disques n'ont pas pu être détectés."
        ));
        assert!(contient(&app, "base udev introuvable"));
    }

    /// Écrit des captures PNG de la fenêtre dans le dossier désigné par `BONK_GUI_CAPTURES`, pour les
    /// regarder. Sans cette variable, le test ne fait rien : aucune image n'est comparée ni versionnée.
    #[test]
    fn captures_facultatives() -> Result<(), Box<dyn std::error::Error>> {
        let Some(dossier) = std::env::var_os("BONK_GUI_CAPTURES") else {
            return Ok(());
        };
        let dossier = std::path::PathBuf::from(dossier);
        let mut app = loaded(sample());
        let theme = app.theme();
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 640.0),
            app.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-cle"))?;
        let _ = app.update(Message::ToggleAll(true));
        let _ = app.update(Message::ToggleInternal(true));
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 640.0),
            app.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-tout"))?;
        let mut erreur = App::default();
        let _ = erreur.update(Message::Loaded(Err("base udev introuvable".to_owned())));
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 640.0),
            erreur.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-erreur"))?;
        Ok(())
    }
}
