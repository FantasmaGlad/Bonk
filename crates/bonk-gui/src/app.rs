//! Fenêtre de Bonk : liste des disques, verdict du filtre de sécurité, détail d'un disque et écriture
//! d'une image.
//!
//! Elle ne contient aucune règle de sécurité ni aucune logique d'écriture : comme `bonk list`, elle
//! présente le verdict de `bonk_hal::evaluate`, et l'écriture lance `bonk write` dans un processus
//! séparé (voir `transfer`), avec tous ses garde-fous. Le disque doit être proposé, un récapitulatif est
//! affiché et le bouton ne s'active qu'après trois secondes (CDC §3.3).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_hal::{
    Disk, Eligibility, FilterOptions, bus_label, enumerate_disks, evaluate, format_size,
    kind_label, state_label, verdict_badge,
};
#[cfg(test)]
use iced::Size;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::{
    Column, button, checkbox, column, container, progress_bar, row, scrollable, text, text_input,
};
use iced::{Element, Fill, Subscription, Task, Theme};

use crate::command::{ViewOptions, equivalent_commands};
use crate::theme;
use crate::transfer::{self, Outcome};

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
    /// Saisie du chemin de l'image à écrire.
    ImageInput(String),
    /// Battement qui rafraîchit le décompte avant d'autoriser l'écriture.
    Clock,
    /// Lance l'écriture (après le délai de réflexion).
    StartWrite,
    CancelWrite,
    /// Referme le résultat d'une écriture.
    DismissWrite,
    /// Ce que rapporte le processus d'écriture.
    Write(transfer::Event),
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
    /// Champ du chemin de l'image, dans le panneau d'écriture du disque sélectionné.
    Image,
    /// Bouton du panneau d'écriture : écrire, annuler ou fermer selon l'état.
    Action,
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

/// Délai de réflexion avant que « Écrire » ne s'active (CDC §3.3).
const WRITE_DELAY: Duration = Duration::from_secs(3);

/// Où en est l'écriture d'une image depuis la fenêtre.
#[derive(Debug, Default)]
enum WriteState {
    #[default]
    Idle,
    /// Récapitulatif affiché, chemin en cours de saisie ; `opened` démarre le délai de réflexion.
    Draft {
        disk_id: String,
        image: String,
        opened: Instant,
    },
    Running {
        disk_id: String,
        pid: Option<u32>,
        phase: String,
        done: u64,
        total: Option<u64>,
        rate: f64,
    },
    Finished {
        disk_id: String,
        outcome: Outcome,
    },
}

impl WriteState {
    fn disk_id(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Draft { disk_id, .. }
            | Self::Running { disk_id, .. }
            | Self::Finished { disk_id, .. } => Some(disk_id),
        }
    }
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
    write: WriteState,
    /// Programme lancé pour écrire : `bonk` lui-même, sauf dans les tests.
    program: Option<PathBuf>,
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
        let image = match &self.write {
            WriteState::Draft { image, .. } => Some(image.as_str()),
            _ => None,
        };
        equivalent_commands(self.view_options(), self.selected.as_deref(), image)
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
                self.sync_write();
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
                self.sync_write();
                self.fix_focus();
                Task::none()
            }
            Message::ToggleInternal(on) => {
                self.show_internal = on;
                self.focus = Some(Focus::Internal);
                self.sync_write();
                self.fix_focus();
                Task::none()
            }
            Message::ToggleVirtual(on) => {
                self.allow_virtual = on;
                self.focus = Some(Focus::Virtual);
                self.sync_write();
                self.fix_focus();
                Task::none()
            }
            Message::ToggleAll(on) => {
                self.show_all = on;
                self.focus = Some(Focus::All);
                self.sync_write();
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
                self.sync_write();
                Task::none()
            }
            Message::Nav(nav) => self.navigate(nav),
            Message::Copy => {
                self.focus = Some(Focus::Copy);
                iced::clipboard::write(self.equivalents().join("\n"))
            }
            Message::ImageInput(text) => {
                if let WriteState::Draft { image, .. } = &mut self.write {
                    *image = text;
                    self.focus = Some(Focus::Image);
                }
                Task::none()
            }
            Message::Clock => Task::none(),
            Message::StartWrite => self.start_write(),
            Message::CancelWrite => {
                self.focus = Some(Focus::Action);
                if let WriteState::Running { pid: Some(pid), .. } = &self.write {
                    log::info!(target: "bonk::gui", pid = *pid; "cancelling the write");
                    transfer::cancel(*pid);
                }
                Task::none()
            }
            Message::DismissWrite => {
                self.write = WriteState::Idle;
                self.sync_write();
                Task::none()
            }
            Message::Write(event) => {
                self.on_write_event(event);
                Task::none()
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
            for (disk, _) in self.entries() {
                order.push(Focus::Disk(disk.id.clone()));
                if self.write.disk_id() == Some(disk.id.as_str()) {
                    if matches!(self.write, WriteState::Draft { .. }) {
                        order.push(Focus::Image);
                    }
                    order.push(Focus::Action);
                }
            }
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
                Some(i)
                    if !matches!(
                        order[i],
                        Focus::Refresh
                            | Focus::Internal
                            | Focus::Virtual
                            | Focus::All
                            | Focus::Copy
                    ) =>
                {
                    // Disque suivant, en sautant le panneau d'écriture du disque courant.
                    (i + 1..order.len())
                        .find(|j| is_disk(&order, *j))
                        .or(Some(i))
                }
                _ => order.iter().position(|item| matches!(item, Focus::Disk(_))),
            },
            // Haut remonte d'un disque ; depuis le bouton de copie, il revient au dernier disque.
            Nav::Up => match current {
                Some(i)
                    if matches!(
                        order[i],
                        Focus::Disk(_) | Focus::Image | Focus::Action | Focus::Copy
                    ) =>
                {
                    // Disque précédent ; depuis les panneaux et le bouton de copie, le disque le plus proche.
                    let from_disk = matches!(order[i], Focus::Disk(_));
                    (0..i)
                        .rev()
                        .find(|j| is_disk(&order, *j))
                        .filter(|j| *j >= 4 || !from_disk)
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
            // Le champ de l'image reçoit les touches lui-même : Espace et Entrée n'y activent rien.
            Some(Focus::Image) | None => return Task::none(),
            Some(Focus::Action) => match self.write {
                WriteState::Draft { .. } => Message::StartWrite,
                WriteState::Running { .. } => Message::CancelWrite,
                WriteState::Finished { .. } => Message::DismissWrite,
                WriteState::Idle => return Task::none(),
            },
        };
        self.apply(message)
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::time::every(REFRESH_EVERY).map(|_| Message::Tick),
            // Une touche prise par un champ de saisie (le chemin de l'image) n'est pas une commande.
            iced::event::listen_with(|event, status, _window| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    repeat,
                    ..
                }) if status == iced::event::Status::Ignored => map_key(&key, modifiers, repeat),
                _ => None,
            }),
        ];
        // Le décompte avant l'activation de « Écrire » se rafraîchit tant que le panneau est ouvert.
        if matches!(self.write, WriteState::Draft { .. }) {
            subscriptions
                .push(iced::time::every(Duration::from_millis(500)).map(|_| Message::Clock));
        }
        Subscription::batch(subscriptions)
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
            text("Détection des disques. Une écriture exige un disque proposé, un récapitulatif et trois secondes de réflexion.")
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
            let mut cards: Vec<Element<'_, Message>> = Vec::new();
            for (disk, verdict) in entries {
                cards.push(self.card(disk, verdict));
                // Le panneau d'écriture est hors de la carte : un clic dedans ne referme pas le disque.
                // Une écriture en cours ou terminée reste visible même si l'on désélectionne le disque :
                // sinon on ne pourrait plus ni la suivre ni l'annuler.
                let show_panel = self.selected.as_deref() == Some(disk.id.as_str())
                    || !matches!(self.write, WriteState::Draft { .. });
                if show_panel && let Some(panel) = self.write_panel(disk) {
                    cards.push(panel);
                }
            }
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

impl App {
    /// Le disque est-il proposé, avec les réglages actuels ?
    fn offered(&self, id: &str) -> bool {
        self.entries()
            .iter()
            .any(|(disk, verdict)| disk.id == id && verdict.is_writable())
    }

    /// Fait suivre le panneau d'écriture à la sélection : il s'ouvre sur un disque proposé, se referme
    /// sinon, et ne bouge jamais pendant qu'une écriture tourne.
    fn sync_write(&mut self) {
        if matches!(self.write, WriteState::Running { .. }) {
            return;
        }
        let wanted = self.selected.clone().filter(|id| self.offered(id));
        match (&self.write, wanted) {
            (current, Some(id)) if current.disk_id() == Some(id.as_str()) => {}
            (_, Some(id)) => {
                self.write = WriteState::Draft {
                    disk_id: id,
                    image: String::new(),
                    opened: Instant::now(),
                };
            }
            (_, None) => self.write = WriteState::Idle,
        }
    }

    fn program(&self) -> PathBuf {
        self.program
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| PathBuf::from("bonk"))
    }

    /// Lance `bonk write` dans un processus séparé, une fois le délai de réflexion écoulé.
    fn start_write(&mut self) -> Task<Message> {
        self.focus = Some(Focus::Action);
        let WriteState::Draft {
            disk_id,
            image,
            opened,
        } = &self.write
        else {
            return Task::none();
        };
        if image.trim().is_empty() || opened.elapsed() < WRITE_DELAY || !self.offered(disk_id) {
            return Task::none();
        }
        let args = transfer::write_args(disk_id, image.trim(), self.view_options());
        log::info!(target: "bonk::gui", disk = disk_id.as_str(); "starting the write");
        let receiver = transfer::spawn(&self.program(), &args);
        self.write = WriteState::Running {
            disk_id: disk_id.clone(),
            pid: None,
            phase: "write".to_owned(),
            done: 0,
            total: None,
            rate: 0.0,
        };
        Task::run(receiver, Message::Write)
    }

    fn on_write_event(&mut self, event: transfer::Event) {
        match (event, &mut self.write) {
            (transfer::Event::Started(new), WriteState::Running { pid, .. }) => *pid = Some(new),
            (
                transfer::Event::Progress {
                    phase: p,
                    done: d,
                    total: t,
                    rate: r,
                },
                WriteState::Running {
                    phase,
                    done,
                    total,
                    rate,
                    ..
                },
            ) => {
                (*phase, *done, *total, *rate) = (p, d, t, r);
            }
            (transfer::Event::Finished(outcome), WriteState::Running { disk_id, .. }) => {
                log::info!(target: "bonk::gui", outcome = format!("{outcome:?}").as_str(); "write finished");
                self.write = WriteState::Finished {
                    disk_id: disk_id.clone(),
                    outcome,
                };
                self.fix_focus();
            }
            _ => {}
        }
    }

    /// Panneau d'écriture du disque sélectionné : récapitulatif (CDC §3.3), saisie de l'image, bouton
    /// désactivé pendant 3 secondes, progression, annulation, résultat.
    fn write_panel<'a>(&'a self, disk: &'a Disk) -> Option<Element<'a, Message>> {
        if self.write.disk_id() != Some(disk.id.as_str()) {
            return None;
        }
        let focused = |target: Focus| self.focus.as_ref() == Some(&target);
        let body: Element<'a, Message> = match &self.write {
            WriteState::Idle => return None,
            WriteState::Draft { image, opened, .. } => {
                let remaining = WRITE_DELAY.saturating_sub(opened.elapsed()).as_secs() + 1;
                let ready = opened.elapsed() >= WRITE_DELAY && !image.trim().is_empty();
                let label = if opened.elapsed() < WRITE_DELAY {
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
                        if labels.is_empty() { String::new() } else { format!("  étiquettes : {}", labels.join(", ")) },
                    ))
                    .size(13),
                    text("Toutes les données de ce disque seront effacées. Cette action est irréversible.")
                        .size(13)
                        .color(theme::SECONDARY),
                    ring(
                        text_input("Chemin de l'image (.img, .iso, .gz, .xz, .zst)", image)
                            .on_input(Message::ImageInput)
                            .on_submit(Message::StartWrite)
                            .padding(8)
                            .size(14)
                            .style(theme::input_style),
                        focused(Focus::Image),
                    ),
                    ring(
                        button(text(label).size(14))
                            .on_press_maybe(ready.then_some(Message::StartWrite))
                            .padding([6, 14])
                            .style(theme::primary_button),
                        focused(Focus::Action),
                    ),
                ]
                .spacing(8)
                .into()
            }
            WriteState::Running {
                phase,
                done,
                total,
                rate,
                ..
            } => {
                let step = if phase == "verify" {
                    "Vérification"
                } else if phase == "flush" {
                    "Vidage des tampons"
                } else {
                    "Écriture"
                };
                let fraction = total
                    .filter(|t| *t > 0)
                    .map_or(0.0, |t| (*done as f32 / t as f32).clamp(0.0, 1.0));
                column![
                    text(format!(
                        "{step} : {}{}  ({}/s)",
                        format_size(*done),
                        total.map_or_else(String::new, |t| format!(" sur {}", format_size(t))),
                        format_size(*rate as u64),
                    ))
                    .size(14),
                    progress_bar(0.0..=1.0, fraction).girth(14),
                    ring(
                        button(text("Annuler").size(14))
                            .on_press(Message::CancelWrite)
                            .padding([6, 14])
                            .style(theme::primary_button),
                        focused(Focus::Action),
                    ),
                ]
                .spacing(8)
                .into()
            }
            WriteState::Finished { outcome, .. } => {
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
                        focused(Focus::Action),
                    ),
                ]
                .spacing(8)
                .into()
            }
        };
        Some(
            container(body)
                .padding(12)
                .width(Fill)
                .style(theme::panel)
                .into(),
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
        // Le chemin saisi n'est jamais journalisé (CDC §9.2) : seulement le fait qu'on saisit.
        Message::ImageInput(text) => (
            "type_image_path",
            format!("{} caractères", text.chars().count()),
        ),
        Message::StartWrite => ("click_write", String::new()),
        Message::CancelWrite => ("click_cancel_write", String::new()),
        Message::DismissWrite => ("click_close_write", String::new()),
        Message::Tick | Message::Clock | Message::Loaded(_) | Message::Write(_) => return None,
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

    fn vieillir_le_panneau(app: &mut App) {
        if let WriteState::Draft { opened, .. } = &mut app.write {
            *opened = Instant::now() - Duration::from_secs(5);
        }
    }

    fn app_avec_panneau() -> App {
        let mut app = loaded(sample());
        // Jamais le vrai binaire : le lancement échoue tout de suite, sans rien écrire.
        app.program = Some(PathBuf::from("/n/existe/pas"));
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        app
    }

    #[test]
    fn le_panneau_d_ecriture_ne_s_ouvre_que_sur_un_disque_propose() {
        let mut app = loaded(sample());
        let _ = app.update(Message::ToggleAll(true));
        let _ = app.update(Message::Select("nvme-systeme".to_owned()));
        assert!(
            matches!(app.write, WriteState::Idle),
            "un disque interdit n'a pas de panneau"
        );
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert!(matches!(&app.write, WriteState::Draft { disk_id, .. } if disk_id == "usb-cle"));
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert!(
            matches!(app.write, WriteState::Idle),
            "désélectionner referme le panneau"
        );
        // Un disque interne n'est proposé (donc écrivable) qu'avec l'option.
        let _ = app.update(Message::Select("sata-libre".to_owned()));
        assert!(matches!(app.write, WriteState::Idle));
        let _ = app.update(Message::ToggleInternal(true));
        assert!(matches!(&app.write, WriteState::Draft { disk_id, .. } if disk_id == "sata-libre"));
        // Retirer l'option referme le panneau : le disque n'est plus proposé.
        let _ = app.update(Message::ToggleInternal(false));
        assert!(matches!(app.write, WriteState::Idle));
    }

    #[test]
    fn l_ecriture_exige_une_image_et_trois_secondes() {
        let mut app = app_avec_panneau();
        // Immédiatement : refusé, même avec une image.
        let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
        let _ = app.update(Message::StartWrite);
        assert!(
            matches!(app.write, WriteState::Draft { .. }),
            "le délai de réflexion n'est pas écoulé"
        );
        vieillir_le_panneau(&mut app);
        // Délai écoulé mais champ vide : refusé.
        let _ = app.update(Message::ImageInput("   ".to_owned()));
        let _ = app.update(Message::StartWrite);
        assert!(matches!(app.write, WriteState::Draft { .. }));
        // Image saisie et délai écoulé : l'écriture démarre.
        let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
        let _ = app.update(Message::StartWrite);
        assert!(matches!(&app.write, WriteState::Running { disk_id, .. } if disk_id == "usb-cle"));
    }

    #[test]
    fn un_disque_devenu_interdit_n_est_pas_ecrit() {
        let mut app = app_avec_panneau();
        let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
        vieillir_le_panneau(&mut app);
        // La clé est débranchée entre-temps : la liste ne la contient plus.
        let _ = app.update(Message::Loaded(Ok(vec![])));
        let _ = app.update(Message::StartWrite);
        assert!(matches!(app.write, WriteState::Idle));
    }

    #[test]
    fn progression_puis_resultat() {
        let mut app = app_avec_panneau();
        let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
        vieillir_le_panneau(&mut app);
        let _ = app.update(Message::StartWrite);
        let _ = app.update(Message::Write(transfer::Event::Started(4242)));
        let _ = app.update(Message::Write(transfer::Event::Progress {
            phase: "write".into(),
            done: 1_000_000,
            total: Some(4_000_000),
            rate: 2e6,
        }));
        assert!(matches!(
            &app.write,
            WriteState::Running {
                pid: Some(4242),
                done: 1_000_000,
                ..
            }
        ));
        assert!(contient(&app, "Écriture : 1,0 Mo sur 4,0 Mo  (2,0 Mo/s)"));
        assert!(contient(&app, "Annuler"));
        // Pendant l'écriture, désélectionner le disque ne cache pas la progression ni l'annulation.
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert!(app.selected.is_none());
        assert!(matches!(app.write, WriteState::Running { .. }));
        assert!(contient(&app, "Annuler"));
        let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Done {
            bytes: 4_000_000,
            verified: true,
            gpt_repaired: true,
        })));
        assert!(matches!(&app.write, WriteState::Finished { .. }));
        assert!(contient(
            &app,
            "ÉCRITURE TERMINÉE : 4,0 Mo écrits, relus et vérifiés, GPT de secours réparé."
        ));
        let _ = app.update(Message::DismissWrite);
        // Fermé : le disque n'est plus sélectionné, rien ne s'ouvre ; le resélectionner rouvre un panneau neuf.
        assert!(matches!(app.write, WriteState::Idle));
        let _ = app.update(Message::Select("usb-cle".to_owned()));
        assert!(matches!(app.write, WriteState::Draft { .. }));
    }

    #[test]
    fn un_echec_s_affiche_avec_son_code_et_sa_commande_d_aide() {
        let mut app = app_avec_panneau();
        app.write = WriteState::Running {
            disk_id: "usb-cle".into(),
            pid: None,
            phase: "write".into(),
            done: 0,
            total: None,
            rate: 0.0,
        };
        let _ = app.update(Message::Write(transfer::Event::Finished(Outcome::Failed {
            code: Some("E4004".into()),
            message: "ouverture du disque refusée".into(),
        })));
        assert!(contient(
            &app,
            "ÉCHEC : ouverture du disque refusée [E4004] : voir « bonk explain E4004 »"
        ));
    }

    #[test]
    fn le_panneau_montre_le_recapitulatif_et_le_decompte() {
        let app = app_avec_panneau();
        assert!(contient(&app, "ÉCRIRE UNE IMAGE SUR CE DISQUE"));
        assert!(contient(
            &app,
            "Toutes les données de ce disque seront effacées. Cette action est irréversible."
        ));
        assert!(contient(
            &app,
            "Cle de test  série 4C530001  16,0 Go  1 partition(s)  étiquettes : CLE"
        ));
        assert!(contient(&app, "Écrire et effacer ce disque (dans 3 s)"));
        let mut vieux = app_avec_panneau();
        vieillir_le_panneau(&mut vieux);
        assert!(contient(
            &vieux,
            "Écrire et effacer ce disque (saisir une image)"
        ));
        let _ = vieux.update(Message::ImageInput("/tmp/x.img".to_owned()));
        assert!(contient(&vieux, "Écrire et effacer ce disque"));
    }

    #[test]
    fn la_commande_equivalente_inclut_l_ecriture_saisie() {
        let mut app = app_avec_panneau();
        assert_eq!(app.equivalents().len(), 2);
        let _ = app.update(Message::ImageInput("/home/u/Mes images/a.iso".to_owned()));
        assert_eq!(
            app.equivalents().last().map(String::as_str),
            Some("bonk write usb-cle '/home/u/Mes images/a.iso' --yes")
        );
    }

    #[test]
    fn le_chemin_saisi_n_est_jamais_journalise() {
        let event = ui_event(&Message::ImageInput("/home/alice/secret.img".to_owned()));
        // Seul le nombre de caractères est connu du journal, jamais le chemin.
        assert_eq!(event, Some(("type_image_path", "22 caractères".to_owned())));
    }

    #[test]
    fn le_clavier_parcourt_le_panneau_et_l_action_respecte_le_delai() {
        let mut app = app_avec_panneau();
        app.focus = Some(Focus::Disk("usb-cle".to_owned()));
        nav(&mut app, Nav::Next);
        assert_eq!(app.focus, Some(Focus::Image));
        nav(&mut app, Nav::Next);
        assert_eq!(app.focus, Some(Focus::Action));
        // Espace sur le bouton avant le délai : sans effet.
        let _ = app.update(Message::ImageInput("/tmp/x.img".to_owned()));
        app.focus = Some(Focus::Action);
        nav(&mut app, Nav::Activate);
        assert!(matches!(app.write, WriteState::Draft { .. }));
        vieillir_le_panneau(&mut app);
        nav(&mut app, Nav::Activate);
        assert!(matches!(app.write, WriteState::Running { .. }));
        // Les flèches sautent le panneau : Bas ne s'arrête pas sur le champ ou le bouton.
        let mut autre = app_avec_panneau();
        let _ = autre.update(Message::ToggleAll(true));
        autre.focus = Some(Focus::Disk("usb-cle".to_owned()));
        nav(&mut autre, Nav::Down);
        assert_eq!(autre.focus, Some(Focus::Disk("sata-libre".to_owned())));
    }

    #[test]
    fn les_touches_prises_par_le_champ_ne_sont_pas_des_commandes() {
        // Le filtre de l'abonnement ne transmet que les touches ignorées par les composants : ici, la
        // correspondance elle-même, qui reste inchangée.
        assert!(matches!(
            map_key(
                &Key::Named(Named::Space),
                keyboard::Modifiers::empty(),
                false
            ),
            Some(Message::Nav(Nav::Activate))
        ));
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
        // Panneau d'écriture : saisie d'une image, puis progression.
        let mut ecriture = loaded(sample());
        let _ = ecriture.update(Message::Select("usb-cle".to_owned()));
        let _ = ecriture.update(Message::ImageInput(
            "/home/alice/images/debian-12.iso".to_owned(),
        ));
        vieillir_le_panneau(&mut ecriture);
        ecriture.focus = Some(Focus::Action);
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 760.0),
            ecriture.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-ecriture"))?;
        ecriture.program = Some(PathBuf::from("/n/existe/pas"));
        let _ = ecriture.update(Message::StartWrite);
        let _ = ecriture.update(Message::Write(transfer::Event::Progress {
            phase: "write".into(),
            done: 1_300_000_000,
            total: Some(2_100_000_000),
            rate: 41e6,
        }));
        Simulator::with_size(
            iced::Settings::default(),
            Size::new(960.0, 760.0),
            ecriture.view(),
        )
        .snapshot(&theme)?
        .matches_image(dossier.join("fenetre-progression"))?;
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
