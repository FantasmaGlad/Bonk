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
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{Column, button, checkbox, column, container, row, scrollable, text};
use iced::{Element, Fill, Subscription, Task, Theme};

use crate::command::{ViewOptions, equivalent_commands};
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
    /// Action au clavier (CDC §8).
    Nav(Nav),
    /// Copie dans le presse-papiers la commande équivalente à la vue (CDC §9.1).
    Copy,
}

/// Actions de la navigation au clavier : chacune a le même effet qu'un clic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Nav {
    Next,
    Previous,
    Activate,
    Up,
    Down,
    Escape,
}

/// Composant qui a le focus clavier.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Focus {
    Refresh,
    Internal,
    Virtual,
    All,
    Disk(String),
    /// Bouton de copie de la commande équivalente, dernier de l'ordre de Tab.
    Copy,
}

/// Identifiant de la liste défilante, pour la suivre quand le focus descend dans la liste.
const LIST: &str = "disques";

/// Touche pressée vers action : Tab et Maj+Tab, Espace et Entrée, flèches haut et bas, Échap.
/// Les combinaisons avec Ctrl, Alt ou Logo sont laissées au système (raccourcis).
pub(crate) fn map_key(key: &Key, modifiers: keyboard::Modifiers, repeat: bool) -> Option<Message> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    let nav = match key {
        Key::Named(Named::Tab) if modifiers.shift() => Nav::Previous,
        Key::Named(Named::Tab) => Nav::Next,
        // Une touche maintenue enfoncée ne doit pas cocher et décocher en boucle.
        Key::Named(Named::Space | Named::Enter) if !repeat => Nav::Activate,
        Key::Named(Named::ArrowUp) => Nav::Up,
        Key::Named(Named::ArrowDown) => Nav::Down,
        Key::Named(Named::Escape) => Nav::Escape,
        _ => return None,
    };
    Some(Message::Nav(nav))
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
    focus: Option<Focus>,
}

fn load() -> Task<Message> {
    Task::perform(
        async { enumerate_disks().map_err(|erreur| erreur.to_string()) },
        Message::Loaded,
    )
}

impl App {
    pub(crate) fn new(initial: ViewOptions) -> (Self, Task<Message>) {
        (
            Self {
                loading: true,
                show_internal: initial.show_internal,
                allow_virtual: initial.allow_virtual,
                show_all: initial.show_all,
                ..Self::default()
            },
            load(),
        )
    }

    /// Applique un message et journalise l'événement (CDC §9.2) : ce que l'utilisateur a fait, ce que
    /// la fenêtre en a tiré, et la commande équivalente à la vue qui en résulte.
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        let event = ui_event(&message);
        let task = self.apply(message);
        if let Some((name, detail)) = event {
            let commands = self.equivalents().join(" ; ");
            let focus = format!("{:?}", self.focus);
            log::debug!(
                target: "bonk::gui",
                event = name, detail = detail.as_str(), focus = focus.as_str(),
                selected = self.selected.as_deref().unwrap_or(""), equivalent = commands.as_str();
                "ui event"
            );
        }
        task
    }

    fn view_options(&self) -> ViewOptions {
        ViewOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
            show_all: self.show_all,
        }
    }

    /// Commandes qui donnent ce que montre la fenêtre en ce moment.
    fn equivalents(&self) -> Vec<String> {
        equivalent_commands(self.view_options(), self.selected.as_deref())
    }

    fn apply(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => {
                self.focus = Some(Focus::Refresh);
                self.reload()
            }
            Message::Tick => {
                log::trace!(target: "bonk::gui", "tick");
                self.reload()
            }
            Message::Loaded(Ok(disks)) => {
                log::debug!(target: "bonk::gui", disks = disks.len(); "enumeration result received");
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
                self.fix_focus();
                Task::none()
            }
            Message::Loaded(Err(erreur)) => {
                log::warn!(target: "bonk::gui", reason = erreur.as_str(); "enumeration failed");
                self.loading = false;
                // Jamais de liste périmée présentée comme valable après un échec.
                self.disks.clear();
                self.selected = None;
                self.error = Some(erreur);
                self.fix_focus();
                Task::none()
            }
            Message::ToggleInternal(on) => {
                self.show_internal = on;
                self.focus = Some(Focus::Internal);
                self.fix_focus();
                Task::none()
            }
            Message::ToggleVirtual(on) => {
                self.allow_virtual = on;
                self.focus = Some(Focus::Virtual);
                self.fix_focus();
                Task::none()
            }
            Message::ToggleAll(on) => {
                self.show_all = on;
                self.focus = Some(Focus::All);
                self.fix_focus();
                Task::none()
            }
            Message::Select(id) => {
                self.focus = Some(Focus::Disk(id.clone()));
                self.selected = if self.selected.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id)
                };
                Task::none()
            }
            Message::Nav(nav) => self.navigate(nav),
            Message::Copy => {
                self.focus = Some(Focus::Copy);
                iced::clipboard::write(self.equivalents().join("\n"))
            }
        }
    }

    /// Lance une énumération, une seule à la fois : le minuteur ne les empile pas si l'une est lente.
    fn reload(&mut self) -> Task<Message> {
        if self.loading {
            Task::none()
        } else {
            self.loading = true;
            load()
        }
    }

    /// Composants focalisables dans l'ordre de Tab : les commandes, puis les disques affichés.
    fn focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::Refresh, Focus::Internal, Focus::Virtual, Focus::All];
        if self.error.is_none() {
            order.extend(
                self.entries()
                    .into_iter()
                    .map(|(disk, _)| Focus::Disk(disk.id.clone())),
            );
        }
        order.push(Focus::Copy);
        order
    }

    /// Le focus ne reste pas sur un disque qui a disparu de la liste (débranché ou masqué).
    fn fix_focus(&mut self) {
        if self
            .focus
            .as_ref()
            .is_some_and(|focus| !self.focus_order().contains(focus))
        {
            self.focus = None;
        }
    }

    fn navigate(&mut self, nav: Nav) -> Task<Message> {
        let order = self.focus_order();
        let current = self
            .focus
            .as_ref()
            .and_then(|focus| order.iter().position(|item| item == focus));
        let last = order.len() - 1;
        let target = match nav {
            Nav::Next => Some(current.map_or(0, |i| if i == last { 0 } else { i + 1 })),
            Nav::Previous => Some(current.map_or(last, |i| if i == 0 { last } else { i - 1 })),
            // Les flèches se déplacent entre les disques ; depuis une commande, Bas entre dans la liste.
            Nav::Down => match current {
                Some(i) if is_disk(&order, i) => Some(i + usize::from(is_disk(&order, i + 1))),
                _ => order.iter().position(|item| matches!(item, Focus::Disk(_))),
            },
            // Haut remonte d'un disque ; depuis le bouton de copie, il revient au dernier disque.
            Nav::Up => match current {
                Some(i) if i > 0 && is_disk(&order, i - 1) && is_disk(&order, i) => Some(i - 1),
                Some(i) if order[i] == Focus::Copy => {
                    i.checked_sub(1).filter(|j| is_disk(&order, *j))
                }
                _ => None,
            },
            Nav::Activate => return self.activate(),
            Nav::Escape => {
                if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.focus = None;
                }
                None
            }
        };
        let Some(index) = target else {
            return Task::none();
        };
        self.focus = Some(order[index].clone());
        // La liste suit le focus : position relative de la carte parmi les disques (approximative si
        // des détails sont ouverts, suffisante pour garder la carte visible).
        let disk_positions: Vec<usize> = (0..order.len()).filter(|i| is_disk(&order, *i)).collect();
        let rank = disk_positions.iter().position(|i| *i == index);
        if let Some(rank) = rank.filter(|_| disk_positions.len() > 1) {
            let relative = rank as f32 / (disk_positions.len() - 1) as f32;
            return iced::widget::operation::snap_to(
                LIST,
                scrollable::RelativeOffset {
                    x: 0.0,
                    y: relative,
                },
            );
        }
        Task::none()
    }

    /// Espace ou Entrée : même effet qu'un clic sur le composant qui a le focus.
    fn activate(&mut self) -> Task<Message> {
        let message = match self.focus.clone() {
            Some(Focus::Refresh) => Message::Refresh,
            Some(Focus::Internal) => Message::ToggleInternal(!self.show_internal),
            Some(Focus::Virtual) => Message::ToggleVirtual(!self.allow_virtual),
            Some(Focus::All) => Message::ToggleAll(!self.show_all),
            Some(Focus::Disk(id)) => Message::Select(id),
            Some(Focus::Copy) => Message::Copy,
            None => return Task::none(),
        };
        self.apply(message)
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(REFRESH_EVERY).map(|_| Message::Tick),
            iced::event::listen_with(|event, _status, _window| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    repeat,
                    ..
                }) => map_key(&key, modifiers, repeat),
                _ => None,
            }),
        ])
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

        let focused = |target: Focus| self.focus.as_ref() == Some(&target);
        let controls = row![
            ring(
                button(text("Actualiser").size(14))
                    .on_press(Message::Refresh)
                    .padding([6, 14])
                    .style(theme::primary_button),
                focused(Focus::Refresh),
            ),
            ring(
                checkbox(self.show_internal)
                    .label("Disques internes non système")
                    .on_toggle(Message::ToggleInternal)
                    .size(18)
                    .style(theme::checkbox_style),
                focused(Focus::Internal),
            ),
            ring(
                checkbox(self.allow_virtual)
                    .label("Disques virtuels (mode test)")
                    .on_toggle(Message::ToggleVirtual)
                    .size(18)
                    .style(theme::checkbox_style),
                focused(Focus::Virtual),
            ),
            ring(
                checkbox(self.show_all)
                    .label("Afficher aussi les disques écartés")
                    .on_toggle(Message::ToggleAll)
                    .size(18)
                    .style(theme::checkbox_style),
                focused(Focus::All),
            ),
        ]
        .spacing(12)
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
            scrollable(Column::with_children(cards).spacing(4).width(Fill))
                .id(LIST)
                .height(Fill)
                .into()
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

        container(
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

/// Vrai si l'élément `i` de l'ordre de Tab est un disque (faux hors bornes).
fn is_disk(order: &[Focus], i: usize) -> bool {
    matches!(order.get(i), Some(Focus::Disk(_)))
}

/// Nom stable et détail d'un événement de l'interface, pour le journal. Le minuteur et les résultats
/// d'énumération sont journalisés ailleurs : pas d'événement utilisateur.
fn ui_event(message: &Message) -> Option<(&'static str, String)> {
    Some(match message {
        Message::Refresh => ("click_refresh", String::new()),
        Message::ToggleInternal(on) => ("toggle_internal", on.to_string()),
        Message::ToggleVirtual(on) => ("toggle_virtual", on.to_string()),
        Message::ToggleAll(on) => ("toggle_all", on.to_string()),
        Message::Select(id) => ("select_disk", id.clone()),
        Message::Nav(nav) => ("key", format!("{nav:?}")),
        Message::Copy => ("click_copy", String::new()),
        Message::Tick | Message::Loaded(_) => return None,
    })
}

/// Entoure un composant de l'indicateur de focus (visible seulement s'il a le focus).
fn ring<'a>(content: impl Into<Element<'a, Message>>, focused: bool) -> Element<'a, Message> {
    container(content)
        .padding(6)
        .style(theme::focus_ring(focused))
        .into()
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

    fn nav(app: &mut App, nav: Nav) {
        let _ = app.update(Message::Nav(nav));
    }

    fn avec_tout_affiche() -> App {
        let mut app = loaded(sample());
        let _ = app.update(Message::ToggleAll(true));
        let _ = app.update(Message::ToggleInternal(true));
        app.focus = None;
        app
    }

    #[test]
    fn tab_parcourt_les_commandes_puis_les_disques_et_reboucle() {
        let mut app = avec_tout_affiche();
        let mut vus = Vec::new();
        for _ in 0..9 {
            nav(&mut app, Nav::Next);
            vus.push(app.focus.clone());
        }
        assert_eq!(
            vus,
            [
                Some(Focus::Refresh),
                Some(Focus::Internal),
                Some(Focus::Virtual),
                Some(Focus::All),
                Some(Focus::Disk("nvme-systeme".to_owned())),
                Some(Focus::Disk("usb-cle".to_owned())),
                Some(Focus::Disk("sata-libre".to_owned())),
                Some(Focus::Copy),
                Some(Focus::Refresh),
            ]
        );
    }

    #[test]
    fn maj_tab_remonte_et_commence_par_le_dernier() {
        let mut app = avec_tout_affiche();
        nav(&mut app, Nav::Previous);
        assert_eq!(app.focus, Some(Focus::Copy));
        nav(&mut app, Nav::Previous);
        assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
        nav(&mut app, Nav::Previous);
        assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
    }

    #[test]
    fn espace_a_le_meme_effet_qu_un_clic() {
        let mut app = loaded(sample());
        app.focus = Some(Focus::Internal);
        nav(&mut app, Nav::Activate);
        assert!(app.show_internal);
        nav(&mut app, Nav::Activate);
        assert!(!app.show_internal);

        app.focus = Some(Focus::All);
        nav(&mut app, Nav::Activate);
        assert!(app.show_all);

        app.focus = Some(Focus::Disk("usb-cle".to_owned()));
        nav(&mut app, Nav::Activate);
        assert_eq!(app.selected.as_deref(), Some("usb-cle"));
        nav(&mut app, Nav::Activate);
        assert_eq!(app.selected, None);

        app.focus = Some(Focus::Refresh);
        nav(&mut app, Nav::Activate);
        assert!(app.loading);
    }

    #[test]
    fn rien_n_est_active_sans_focus() {
        let mut app = loaded(sample());
        nav(&mut app, Nav::Activate);
        assert!(!app.show_all && !app.show_internal && app.selected.is_none() && !app.loading);
    }

    #[test]
    fn les_fleches_se_deplacent_entre_les_disques() {
        let mut app = avec_tout_affiche();
        app.focus = Some(Focus::Refresh);
        // Bas depuis une commande entre dans la liste.
        nav(&mut app, Nav::Down);
        assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
        nav(&mut app, Nav::Down);
        nav(&mut app, Nav::Down);
        assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
        // Pas de bouclage en bas ni en haut.
        nav(&mut app, Nav::Down);
        assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
        nav(&mut app, Nav::Up);
        nav(&mut app, Nav::Up);
        nav(&mut app, Nav::Up);
        assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
    }

    #[test]
    fn echap_referme_le_detail_puis_retire_le_focus() {
        let mut app = loaded(sample());
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        nav(&mut app, Nav::Escape);
        assert_eq!(app.selected, None);
        assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
        nav(&mut app, Nav::Escape);
        assert_eq!(app.focus, None);
    }

    #[test]
    fn le_focus_quitte_un_disque_qui_disparait() {
        let mut app = avec_tout_affiche();
        app.focus = Some(Focus::Disk("sata-libre".to_owned()));
        // La case « écartés » masque le disque interne : le focus ne reste pas sur un composant absent.
        let _ = app.update(Message::ToggleInternal(false));
        assert_eq!(app.focus, Some(Focus::Internal));
        app.focus = Some(Focus::Disk("sata-libre".to_owned()));
        let _ = app.update(Message::Loaded(Ok(vec![])));
        assert_eq!(app.focus, None);
        // Un échec de détection retire aussi les disques de l'ordre de Tab.
        let mut app = avec_tout_affiche();
        app.focus = Some(Focus::Disk("usb-cle".to_owned()));
        let _ = app.update(Message::Loaded(Err("panne".to_owned())));
        assert_eq!(app.focus, None);
        nav(&mut app, Nav::Next);
        nav(&mut app, Nav::Previous);
        assert_eq!(app.focus, Some(Focus::Copy));
    }

    #[test]
    fn un_clic_donne_le_focus_au_composant() {
        let mut app = loaded(sample());
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
        let _ = app.update(Message::ToggleAll(true));
        assert_eq!(app.focus, Some(Focus::All));
        // Le minuteur ne touche pas au focus.
        let _ = app.update(Message::Tick);
        assert_eq!(app.focus, Some(Focus::All));
    }

    #[test]
    fn touches_vers_actions() {
        use keyboard::Modifiers as M;
        let tab = Key::Named(Named::Tab);
        let nav_de = |key: &Key, mods: M, repeat: bool| match map_key(key, mods, repeat) {
            Some(Message::Nav(n)) => Some(n),
            _ => None,
        };
        assert_eq!(nav_de(&tab, M::empty(), false), Some(Nav::Next));
        assert_eq!(nav_de(&tab, M::SHIFT, false), Some(Nav::Previous));
        assert_eq!(
            nav_de(&Key::Named(Named::Space), M::empty(), false),
            Some(Nav::Activate)
        );
        assert_eq!(
            nav_de(&Key::Named(Named::Enter), M::empty(), false),
            Some(Nav::Activate)
        );
        assert_eq!(
            nav_de(&Key::Named(Named::ArrowDown), M::empty(), false),
            Some(Nav::Down)
        );
        assert_eq!(
            nav_de(&Key::Named(Named::ArrowUp), M::empty(), false),
            Some(Nav::Up)
        );
        assert_eq!(
            nav_de(&Key::Named(Named::Escape), M::empty(), false),
            Some(Nav::Escape)
        );
        // Raccourcis du système laissés tranquilles, répétition de touche ignorée pour activer.
        assert_eq!(nav_de(&tab, M::CTRL, false), None);
        assert_eq!(nav_de(&tab, M::ALT, false), None);
        assert_eq!(nav_de(&Key::Named(Named::Space), M::empty(), true), None);
        assert_eq!(nav_de(&tab, M::empty(), true), Some(Nav::Next));
        assert_eq!(nav_de(&Key::Character("a".into()), M::empty(), false), None);
    }

    #[test]
    fn haut_depuis_le_bouton_de_copie_revient_au_dernier_disque() {
        let mut app = avec_tout_affiche();
        app.focus = Some(Focus::Copy);
        nav(&mut app, Nav::Up);
        assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
        // Bas depuis le dernier disque n'entre pas dans le bouton : c'est le rôle de Tab.
        nav(&mut app, Nav::Down);
        assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
        // Sans disque affiché, Haut depuis la copie ne fait rien.
        let mut vide = loaded(vec![]);
        vide.focus = Some(Focus::Copy);
        nav(&mut vide, Nav::Up);
        assert_eq!(vide.focus, Some(Focus::Copy));
    }

    #[test]
    fn la_commande_equivalente_suit_la_vue() {
        let mut app = loaded(sample());
        assert_eq!(app.equivalents(), ["bonk list"]);
        let _ = app.update(Message::ToggleAll(true));
        let _ = app.update(Message::ToggleInternal(true));
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert_eq!(
            app.equivalents(),
            [
                "bonk list --show-internal --all",
                "bonk info usb-cle --show-internal"
            ]
        );
        assert!(contient(
            &app,
            "bonk list --show-internal --all\nbonk info usb-cle --show-internal"
        ));
        assert!(contient(&app, "Commande équivalente"));
    }

    #[test]
    fn le_bouton_copier_prend_le_focus_et_garde_la_vue() {
        let mut app = loaded(sample());
        let _ = app.update(Message::Copy);
        assert_eq!(app.focus, Some(Focus::Copy));
        assert_eq!(app.equivalents(), ["bonk list"]);
    }

    #[test]
    fn evenements_de_l_interface_journalises() {
        assert_eq!(
            ui_event(&Message::Refresh),
            Some(("click_refresh", String::new()))
        );
        assert_eq!(
            ui_event(&Message::ToggleInternal(true)),
            Some(("toggle_internal", "true".to_owned()))
        );
        assert_eq!(
            ui_event(&Message::Select("usb-cle".to_owned())),
            Some(("select_disk", "usb-cle".to_owned()))
        );
        assert_eq!(
            ui_event(&Message::Nav(Nav::Next)),
            Some(("key", "Next".to_owned()))
        );
        assert_eq!(
            ui_event(&Message::Copy),
            Some(("click_copy", String::new()))
        );
        // Le minuteur et les résultats ont leur propre journal.
        assert_eq!(ui_event(&Message::Tick), None);
        assert_eq!(ui_event(&Message::Loaded(Ok(vec![]))), None);
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
        // Focus clavier sur la deuxième carte, détail fermé.
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        nav(&mut app, Nav::Escape);
        nav(&mut app, Nav::Previous);
        nav(&mut app, Nav::Previous);
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 640.0),
            app.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-focus"))?;
        app.focus = Some(Focus::Virtual);
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 640.0),
            app.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-focus-case"))?;
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
