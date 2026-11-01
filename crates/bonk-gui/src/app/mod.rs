//! Fenêtre de Bonk : liste des disques, verdict du filtre de sécurité, détail d'un disque et écriture
//! d'une image.
//!
//! Elle ne contient aucune règle de sécurité ni aucune logique d'écriture : comme `bonk list`, elle
//! présente le verdict de `bonk_hal::evaluate`, et l'écriture lance `bonk write` dans un processus
//! séparé (voir `transfer`), avec tous ses garde-fous. Le disque doit être proposé, un récapitulatif est
//! affiché et le bouton ne s'active qu'après trois secondes.

mod catalog;
mod navigation;
#[cfg(test)]
mod tests;
mod update;
mod view;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_hal::{Disk, Eligibility, FilterOptions, Warning, enumerate_disks, evaluate};
#[cfg(test)]
use iced::Size;
use iced::keyboard;
use iced::{Subscription, Task, Theme};

use bonk_core::write::Phase;

use crate::command::{ViewOptions, WriteView, equivalent_commands};
use crate::theme;
use crate::transfer::{self, Outcome};

use navigation::map_key;

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
    /// Action au clavier.
    Nav(Nav),
    /// Copie dans le presse-papiers la commande équivalente à la vue.
    Copy,
    /// Saisie du chemin de l'image à écrire.
    ImageInput(String),
    /// Saisie de l'empreinte, ou du chemin ou de l'adresse d'un fichier d'empreintes.
    HashInput(String),
    /// Écrire les octets tels quels, sans refuser une image que l'analyse ne juge pas amorçable.
    ToggleRaw(bool),
    /// Table MBR pour une clé Windows.
    ToggleMbr(bool),
    /// Ouvre le sélecteur de fichier du système pour choisir l'image.
    BrowseImage,
    /// Réponse du sélecteur ; `None` : fermé sans choix.
    ImagePicked(Option<PathBuf>),
    /// Ouvre ou referme le menu des images.
    ToggleMenu,
    MenuCategory(Option<String>),
    MenuSystem(Option<String>),
    CatalogLoaded(Result<Vec<catalog::System>, String>),
    /// Une version choisie : adresse, empreinte et réglages automatiques.
    MenuChose(catalog::Choice),
    /// Montre ou cache l'image manuelle et les options.
    ToggleAdvanced,
    /// Battement qui rafraîchit le décompte avant d'autoriser l'écriture.
    Clock,
    /// Lance l'écriture (après le délai de réflexion).
    StartWrite,
    CancelWrite,
    /// Referme le résultat d'une écriture.
    DismissWrite,
    /// Dans la demande de confirmation d'un disque interne : effacer quand même.
    ConfirmInternal,
    /// Dans la demande de confirmation d'un disque interne : renoncer.
    CloseInternal,
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
    /// Bouton qui ouvre le sélecteur de fichier, à côté du champ de l'image.
    Browse,
    /// Champ de l'empreinte.
    Hash,
    /// Menu des images.
    Menu,
    /// Bouton qui montre l'image manuelle et les options.
    Advanced,
    /// Case de l'écriture brute.
    Raw,
    /// Case de la table MBR.
    Mbr,
    /// Bouton du panneau d'écriture : écrire, annuler ou fermer selon l'état.
    Action,
    /// Bouton de copie de la commande équivalente, dernier de l'ordre de Tab.
    Copy,
}

/// Délai de réflexion avant que « Écrire » ne s'active.
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
        hash: String,
        raw: bool,
        mbr: bool,
        opened: Instant,
    },
    Running {
        disk_id: String,
        pid: Option<u32>,
        phase: Phase,
        done: u64,
        total: Option<u64>,
        rate: f64,
    },
    Finished {
        disk_id: String,
        outcome: Outcome,
    },
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
    /// La demande de confirmation d'un disque interne est à l'écran.
    confirming_internal: bool,
    catalog: catalog::CatalogState,
    menu_filter: catalog::MenuFilter,
    menu_open: bool,
    /// Ligne du menu désignée au clavier.
    menu_index: usize,
    /// Image manuelle et options visibles.
    advanced: bool,
    /// À l'ouverture de la fenêtre, un seul disque proposé se sélectionne seul, une fois pour chaque disque :
    /// écrire reste un geste explicite, avec le disque nommé à l'écran.
    auto_select: bool,
    auto_selected: Option<String>,
    /// Programme lancé pour écrire : `bonk` lui-même, sauf dans les tests.
    program: Option<PathBuf>,
}

fn load() -> Task<Message> {
    Task::perform(
        async { enumerate_disks().map_err(|erreur| erreur.to_string()) },
        Message::Loaded,
    )
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
        // Le détail de l'événement ne porte que la longueur du chemin saisi.
        Message::ImageInput(text) => (
            "type_image_path",
            format!("{} caractères", text.chars().count()),
        ),
        Message::HashInput(text) => ("type_hash", format!("{} caractères", text.chars().count())),
        Message::ToggleRaw(on) => ("toggle_raw", on.to_string()),
        Message::ToggleMbr(on) => ("toggle_mbr", on.to_string()),
        Message::BrowseImage => ("click_browse", String::new()),
        // Comme pour la saisie, le détail de l'événement ne porte pas le chemin choisi.
        Message::ImagePicked(path) => (
            "image_picked",
            if path.is_some() { "choisie" } else { "aucune" }.to_owned(),
        ),
        Message::ToggleMenu => ("click_menu", String::new()),
        Message::MenuCategory(value) | Message::MenuSystem(value) => {
            ("menu_filter", value.clone().unwrap_or_default())
        }
        Message::MenuChose(choice) => ("menu_chose", choice.label.clone()),
        Message::ToggleAdvanced => ("click_advanced", String::new()),
        Message::StartWrite => ("click_write", String::new()),
        Message::CancelWrite => ("click_cancel_write", String::new()),
        Message::DismissWrite => ("click_close_write", String::new()),
        Message::ConfirmInternal => ("click_confirm_internal", String::new()),
        Message::CloseInternal => ("click_close_internal", String::new()),
        Message::Tick
        | Message::Clock
        | Message::Loaded(_)
        | Message::Write(_)
        | Message::CatalogLoaded(_) => return None,
    })
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

impl App {
    pub(crate) fn new(initial: ViewOptions) -> (Self, Task<Message>) {
        (
            Self {
                loading: true,
                auto_select: true,
                show_internal: initial.show_internal,
                allow_virtual: initial.allow_virtual,
                show_all: initial.show_all,
                ..Self::default()
            },
            load(),
        )
    }

    /// Applique un message et journalise l'événement : ce que l'utilisateur a fait, ce que
    /// la fenêtre en a tiré, et la commande équivalente à la vue qui en résulte.
    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        let event = ui_event(&message);
        let task = self.apply(message);
        if let Some((name, detail)) = event {
            let commands = self.logged_equivalents();
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

    /// Commandes pour le journal : sans l'image, dont le chemin peut dire qui l'on est.
    fn logged_equivalents(&self) -> String {
        equivalent_commands(self.view_options(), self.selected.as_deref(), None).join(" ; ")
    }

    /// Commandes qui donnent ce que montre la fenêtre en ce moment.
    fn equivalents(&self) -> Vec<String> {
        let write = match &self.write {
            WriteState::Draft {
                image,
                hash,
                raw,
                mbr,
                ..
            } => Some(WriteView {
                image,
                hash,
                raw: *raw,
                mbr: *mbr,
            }),
            _ => None,
        };
        equivalent_commands(self.view_options(), self.selected.as_deref(), write)
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
                // Sous Wayland, winit 0.30 ne remet aucun fichier déposé : le sélecteur reste la voie sûre.
                iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                    Some(Message::ImagePicked(Some(path)))
                }
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

    /// Le disque est-il proposé, avec les réglages actuels ?
    fn offered(&self, id: &str) -> bool {
        self.entries()
            .iter()
            .any(|(disk, verdict)| disk.id == id && verdict.is_writable())
    }

    /// Le disque est-il proposé en tant que disque interne ?
    fn is_internal(&self, id: &str) -> bool {
        self.entries().iter().any(|(disk, verdict)| {
            disk.id == id
                && *verdict
                    == (Eligibility::Offered {
                        warning: Warning::InternalDisk,
                    })
        })
    }

    /// Le disque dont la confirmation est demandée : le panneau peut avoir changé depuis l'ouverture.
    fn internal_to_confirm(&self) -> Option<&Disk> {
        let id = self.write.disk_id().filter(|_| {
            self.confirming_internal && matches!(self.write, WriteState::Draft { .. })
        })?;
        self.disks
            .iter()
            .find(|disk| disk.id == id && self.is_internal(id))
    }
}
