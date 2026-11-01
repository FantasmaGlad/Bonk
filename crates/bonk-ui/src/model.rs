//! L'état de l'interface et ses règles, sans dessin ni système : chaque interface le dessine à sa façon et
//! accomplit les [`Effect`] demandés. Elle ne contient aucune règle de sécurité ni logique d'écriture : le
//! verdict vient de `bonk_hal::evaluate`, et un disque doit être proposé, un récapitulatif affiché et le bouton
//! n'est actif qu'après trois secondes.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_core::write::Phase;
use bonk_hal::{Disk, Eligibility, FilterOptions, Lang, Warning, evaluate};

use crate::command::{ViewOptions, WriteView, equivalent_commands};
use crate::event::{Event, Outcome};

/// Intervalle d'actualisation automatique : un disque branché apparaît sans geste de l'utilisateur.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// Ce que l'interface doit accomplir après un message ; le résultat revient par un autre message.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Enumerate,
    LoadCatalog,
    DetectTor,
    Copy(String),
    PickImage,
    SaveLanguage(Lang),
    /// Arguments de `bonk write` ; une interface sans ligne de commande les ignore.
    StartWrite(Vec<String>),
    CancelWrite(u32),
    /// Position relative, de 0 à 1, à garder visible.
    ScrollList(f32),
    ScrollMenu(f32),
}

pub type Effects = Vec<Effect>;

#[derive(Debug, Clone)]
pub enum Message {
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
    TorDetected(Option<String>),
    ToggleTor(bool),
    SwitchLanguage,
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
    CatalogLoaded(Result<Vec<crate::catalog::System>, String>),
    /// Une version choisie : adresse, empreinte et réglages automatiques.
    MenuChose(crate::catalog::Choice),
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
    Write(Event),
}

/// Actions de la navigation au clavier : chacune a le même effet qu'un clic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Next,
    Previous,
    Activate,
    Up,
    Down,
    Escape,
}

/// Composant qui a le focus clavier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
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
    Tor,
    Language,
    Raw,
    /// Case de la table MBR.
    Mbr,
    /// Bouton du panneau d'écriture : écrire, annuler ou fermer selon l'état.
    Action,
    /// Bouton de copie de la commande équivalente, dernier de l'ordre de Tab.
    Copy,
}

/// Délai de réflexion avant que « Écrire » ne s'active.
pub const WRITE_DELAY: Duration = Duration::from_secs(3);

/// Où en est l'écriture d'une image depuis la fenêtre.
#[derive(Debug, Default)]
pub enum WriteState {
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
pub struct Model {
    pub disks: Vec<Disk>,
    pub loading: bool,
    /// Une première réponse est arrivée : la détection périodique ne se montre plus à l'écran.
    pub answered: bool,
    pub error: Option<String>,
    pub show_internal: bool,
    pub allow_virtual: bool,
    pub show_all: bool,
    pub selected: Option<String>,
    pub focus: Option<Focus>,
    pub write: WriteState,
    /// La demande de confirmation d'un disque interne est à l'écran.
    pub confirming_internal: bool,
    pub catalog: crate::catalog::CatalogState,
    pub menu_filter: crate::catalog::MenuFilter,
    /// Adresse d'un Tor local, proposé dans les options ; jamais utilisé sans la case.
    pub tor: Option<String>,
    pub lang: Lang,
    pub use_tor: bool,
    pub menu_open: bool,
    /// Ligne du menu désignée au clavier.
    pub menu_index: usize,
    /// Image manuelle et options visibles.
    pub advanced: bool,
    /// À l'ouverture de la fenêtre, un seul disque proposé se sélectionne seul, une fois pour chaque disque :
    /// écrire reste un geste explicite, avec le disque nommé à l'écran.
    pub auto_select: bool,
    pub auto_selected: Option<String>,
}

pub fn ui_event(message: &Message) -> Option<(&'static str, String)> {
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
        Message::ToggleTor(on) => ("toggle_tor", on.to_string()),
        Message::SwitchLanguage => ("switch_language", String::new()),
        Message::TorDetected(found) => ("tor_detected", found.is_some().to_string()),
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
    pub fn disk_id(&self) -> Option<&str> {
        match self {
            Self::Idle => None,
            Self::Draft { disk_id, .. }
            | Self::Running { disk_id, .. }
            | Self::Finished { disk_id, .. } => Some(disk_id),
        }
    }
}

impl Model {
    /// Une fenêtre qui vient de s'ouvrir, et ce qu'elle doit faire d'abord.
    #[must_use]
    pub fn new(initial: ViewOptions, lang: Lang) -> (Self, Effects) {
        (
            Self {
                lang,
                loading: true,
                auto_select: true,
                show_internal: initial.show_internal,
                allow_virtual: initial.allow_virtual,
                show_all: initial.show_all,
                ..Self::default()
            },
            vec![Effect::Enumerate],
        )
    }

    /// Applique un message et journalise l'événement : ce que l'utilisateur a fait, ce que
    /// la fenêtre en a tiré, et la commande équivalente à la vue qui en résulte.
    pub fn update(&mut self, message: Message) -> Effects {
        let event = ui_event(&message);
        let effects = self.apply(message);
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
        effects
    }

    pub fn view_options(&self) -> ViewOptions {
        ViewOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
            show_all: self.show_all,
        }
    }

    /// Commandes pour le journal : sans l'image, dont le chemin peut dire qui l'on est.
    pub fn logged_equivalents(&self) -> String {
        equivalent_commands(self.view_options(), self.selected.as_deref(), None).join(" ; ")
    }

    /// Commandes qui donnent ce que montre la fenêtre en ce moment.
    pub fn equivalents(&self) -> Vec<String> {
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

    /// Disques à afficher avec leur verdict, selon les réglages.
    pub fn entries(&self) -> Vec<(&Disk, Eligibility)> {
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
    pub fn offered(&self, id: &str) -> bool {
        self.entries()
            .iter()
            .any(|(disk, verdict)| disk.id == id && verdict.is_writable())
    }

    /// Le disque est-il proposé en tant que disque interne ?
    pub fn is_internal(&self, id: &str) -> bool {
        self.entries().iter().any(|(disk, verdict)| {
            disk.id == id
                && *verdict
                    == (Eligibility::Offered {
                        warning: Warning::InternalDisk,
                    })
        })
    }

    /// Le disque dont la confirmation est demandée : le panneau peut avoir changé depuis l'ouverture.
    pub fn internal_to_confirm(&self) -> Option<&Disk> {
        let id = self.write.disk_id().filter(|_| {
            self.confirming_internal && matches!(self.write, WriteState::Draft { .. })
        })?;
        self.disks
            .iter()
            .find(|disk| disk.id == id && self.is_internal(id))
    }

    pub fn status_line(&self, shown: usize) -> String {
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
        if self.loading && !self.answered {
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
}
