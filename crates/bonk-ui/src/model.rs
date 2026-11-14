use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_core::write::Phase;
use bonk_hal::{Disk, Eligibility, FilterOptions, Lang, Warning, evaluate};

use crate::command::{ViewOptions, WriteView, equivalent_commands};
use crate::event::{Event, Outcome};

/// A plugged disk appears without a user action.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// What the interface must do after a message; the result comes back as another message.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Enumerate,
    LoadCatalog,
    DetectTor,
    Copy(String),
    PickImage,
    /// Kept for the next start (`bonk config set`); takes effect now.
    SaveSetting {
        key: &'static str,
        value: &'static str,
    },
    /// Kept, then the interface starts again: language and theme never change while it runs.
    Relaunch {
        key: &'static str,
        value: &'static str,
    },
    /// `bonk write` arguments; an interface without a command line ignores them.
    StartWrite(Vec<String>),
    CancelWrite(u32),
    ScrollList(f32),
    ScrollMenu(f32),
    /// `bonk vault` arguments, run as `bonk write` is.
    RunVault(Vec<String>),
    PickVaultIso,
    /// `bonk cache list --json`, answered by [`crate::library::LibraryMessage::Listed`].
    ListKept,
    /// `bonk cache clean <SHA256>`, answered by [`crate::library::LibraryMessage::Removed`].
    RemoveKept(String),
}

pub type Effects = Vec<Effect>;

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    /// The error is already text (`EnumerateError` is not `Clone`).
    Loaded(Result<Vec<Disk>, String>),
    ToggleInternal(bool),
    ToggleVirtual(bool),
    ToggleAll(bool),
    /// Selects or deselects a disk, by id.
    Select(String),
    /// Shows or hides the technical details of the selected disk.
    ToggleDetails,
    Nav(Nav),
    Copy,
    ImageInput(String),
    HashInput(String),
    ToggleRaw(bool),
    TorDetected(Option<String>),
    ToggleTor(bool),
    ToggleMbr(bool),
    BrowseImage,
    /// `None`: closed without a choice.
    ImagePicked(Option<PathBuf>),
    ToggleMenu,
    MenuCategory(Option<String>),
    MenuBase(Option<String>),
    MenuSystem(Option<String>),
    CatalogLoaded(Result<crate::catalog::Shelf, String>),
    /// Address, hash and automatic settings.
    MenuChose(crate::catalog::Choice),
    ToggleAdvanced,
    /// Refreshes the countdown before the write is allowed.
    Clock,
    /// After the delay to think.
    StartWrite,
    CancelWrite,
    DismissWrite,
    /// Internal disk confirmation: erase anyway.
    ConfirmInternal,
    /// Internal disk confirmation: give up.
    CloseInternal,
    Write(Event),
    Vault(crate::vault::VaultMessage),
    Settings(crate::settings::SettingsMessage),
    Library(crate::library::LibraryMessage),
    /// A `bonk://install/` link opened while the window runs (macOS sends it to the running process).
    RequestImage(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Next,
    Previous,
    Activate,
    Up,
    Down,
    Escape,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Focus {
    /// Opens the settings page.
    Settings,
    CloseSettings,
    /// Opens the library page.
    Library,
    CloseLibrary,
    /// Write or remove the kept image with this hash.
    KeptWrite(String),
    KeptRemove(String),
    Language(crate::settings::LangChoice),
    Theme(crate::settings::ThemeChoice),
    Internal,
    Virtual,
    All,
    Offline,
    Disk(String),
    /// The technical details disclosure of the selected disk.
    Details,
    Image,
    Browse,
    Hash,
    Menu,
    Advanced,
    Tor,
    Raw,
    Mbr,
    /// Write panel button: write, cancel or close depending on the state.
    Action,
    Copy,
}

/// Delay to think before Write enables.
pub const WRITE_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub enum WriteState {
    #[default]
    Idle,
    /// `opened` starts the delay to think.
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
    /// A first answer arrived: periodic detection no longer shows on screen.
    pub answered: bool,
    pub error: Option<String>,
    pub show_internal: bool,
    pub allow_virtual: bool,
    pub show_all: bool,
    pub selected: Option<String>,
    /// Technical details of the selected disk shown; closed for each new selection.
    pub details: bool,
    pub focus: Option<Focus>,
    pub write: WriteState,
    pub confirming_internal: bool,
    pub catalog: crate::catalog::CatalogState,
    /// Never reach the network: the catalog is neither read nor shown, and a web address is refused.
    pub offline: bool,
    /// Labels of the usage categories and bases, from the catalog once read.
    pub terms: crate::terms::Terms,
    pub menu_filter: crate::catalog::MenuFilter,
    /// A local Tor, offered in the options; never used without the box.
    pub tor: Option<String>,
    pub lang: Lang,
    pub use_tor: bool,
    pub menu_open: bool,
    pub menu_index: usize,
    pub advanced: bool,
    /// When the window opens a single offered disk selects itself, once per disk: writing stays an explicit
    /// act, with the disk named on screen.
    pub auto_select: bool,
    pub auto_selected: Option<String>,
    /// Catalog image the window was opened for (`--image`, a link): it fills each draft until the user
    /// chooses something else. It only ever names a catalog entry, and never starts a write.
    pub requested: Option<String>,
    pub vault: crate::vault::VaultState,
    pub library: crate::library::LibraryState,
    pub screen: crate::settings::Screen,
    pub lang_choice: crate::settings::LangChoice,
    pub theme_choice: crate::settings::ThemeChoice,
    /// The system's answer for [`crate::settings::ThemeChoice::System`]; `None` until it comes.
    pub system_dark: Option<bool>,
}

/// Where the image the window was opened for stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested<'a> {
    Waiting,
    Ready(&'a crate::catalog::Choice),
    /// In the catalog, but not a file Bonk can fetch and write to a drive.
    NotWritable(&'a crate::catalog::Choice),
    Unknown,
    CatalogFailed,
}

pub fn ui_event(message: &Message) -> Option<(&'static str, String)> {
    Some(match message {
        Message::ToggleInternal(on) => ("toggle_internal", on.to_string()),
        Message::ToggleVirtual(on) => ("toggle_virtual", on.to_string()),
        Message::ToggleAll(on) => ("toggle_all", on.to_string()),
        Message::Select(id) => ("select_disk", id.clone()),
        Message::ToggleDetails => ("toggle_details", String::new()),
        Message::Nav(nav) => ("key", format!("{nav:?}")),
        Message::Copy => ("click_copy", String::new()),
        // The event detail only carries the path length.
        Message::ImageInput(text) => (
            "type_image_path",
            format!("{} caractères", text.chars().count()),
        ),
        Message::HashInput(text) => ("type_hash", format!("{} caractères", text.chars().count())),
        Message::ToggleRaw(on) => ("toggle_raw", on.to_string()),
        Message::ToggleTor(on) => ("toggle_tor", on.to_string()),
        Message::TorDetected(found) => ("tor_detected", found.is_some().to_string()),
        Message::ToggleMbr(on) => ("toggle_mbr", on.to_string()),
        Message::BrowseImage => ("click_browse", String::new()),
        // As for typing, the event detail does not carry the chosen path.
        Message::ImagePicked(path) => (
            "image_picked",
            if path.is_some() { "choisie" } else { "aucune" }.to_owned(),
        ),
        Message::ToggleMenu => ("click_menu", String::new()),
        Message::RequestImage(id) => ("open_link", id.clone()),
        Message::MenuCategory(value) | Message::MenuBase(value) | Message::MenuSystem(value) => {
            ("menu_filter", value.clone().unwrap_or_default())
        }
        Message::MenuChose(choice) => ("menu_chose", choice.label.clone()),
        Message::ToggleAdvanced => ("click_advanced", String::new()),
        Message::StartWrite => ("click_write", String::new()),
        Message::CancelWrite => ("click_cancel_write", String::new()),
        Message::DismissWrite => ("click_close_write", String::new()),
        Message::ConfirmInternal => ("click_confirm_internal", String::new()),
        Message::CloseInternal => ("click_close_internal", String::new()),
        Message::Vault(message) => return vault_event(message),
        Message::Settings(message) => return settings_event(*message),
        Message::Library(message) => return library_event(message),
        Message::Tick
        | Message::Clock
        | Message::Loaded(_)
        | Message::Write(_)
        | Message::CatalogLoaded(_) => return None,
    })
}

fn vault_event(message: &crate::vault::VaultMessage) -> Option<(&'static str, String)> {
    use crate::vault::VaultMessage;
    Some(match message {
        VaultMessage::Open => ("click_vault", String::new()),
        VaultMessage::Close => ("click_close_vault", String::new()),
        VaultMessage::Prepare => ("click_prepare_vault", String::new()),
        VaultMessage::ConfirmPrepare => ("click_confirm_vault", String::new()),
        VaultMessage::Browse => ("click_add_iso", String::new()),
        // As for the image, the event detail does not carry the chosen path.
        VaultMessage::Picked(path) => ("iso_picked", path.is_some().to_string()),
        VaultMessage::Update => ("click_update_vault", String::new()),
        VaultMessage::Cancel => ("click_cancel_vault", String::new()),
        VaultMessage::Event(_) => return None,
    })
}

fn settings_event(message: crate::settings::SettingsMessage) -> Option<(&'static str, String)> {
    use crate::settings::SettingsMessage;
    Some(match message {
        SettingsMessage::Open => ("click_settings", String::new()),
        SettingsMessage::Close => ("click_close_settings", String::new()),
        SettingsMessage::Language(choice) => ("choose_language", choice.config_value().to_owned()),
        SettingsMessage::Theme(choice) => ("choose_theme", choice.config_value().to_owned()),
        SettingsMessage::Offline(on) => ("toggle_offline", on.to_string()),
        SettingsMessage::SystemDark(_) => return None,
    })
}

fn library_event(message: &crate::library::LibraryMessage) -> Option<(&'static str, String)> {
    use crate::library::LibraryMessage;
    Some(match message {
        LibraryMessage::Open => ("click_library", String::new()),
        LibraryMessage::Close => ("click_close_library", String::new()),
        LibraryMessage::Remove(sha256) => ("click_remove_kept", sha256.clone()),
        LibraryMessage::Write(sha256) => ("click_write_kept", sha256.clone()),
        LibraryMessage::Listed(_) | LibraryMessage::Removed(_) => return None,
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
                offline: initial.offline,
                advanced: initial.offline,
                ..Self::default()
            },
            vec![Effect::Enumerate],
        )
    }

    /// Logs the event: what the user did, what the window made of it, and the equivalent command of the
    /// resulting view.
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

    pub fn requested(&self) -> Option<(&str, Requested<'_>)> {
        use crate::catalog::{CatalogState, ChoiceSource, find_id};
        let id = self.requested.as_deref()?;
        let state = match &self.catalog {
            CatalogState::NotLoaded | CatalogState::Loading => Requested::Waiting,
            CatalogState::Failed(_) => Requested::CatalogFailed,
            CatalogState::Ready(systems) => match find_id(systems, id) {
                Some((_, choice)) if matches!(choice.source, ChoiceSource::Download { .. }) => {
                    Requested::Ready(choice)
                }
                Some((_, choice)) => Requested::NotWritable(choice),
                None => Requested::Unknown,
            },
        };
        Some((id, state))
    }

    pub fn view_options(&self) -> ViewOptions {
        ViewOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
            show_all: self.show_all,
            offline: self.offline,
        }
    }

    /// Without the image, whose path may say who the user is.
    pub fn logged_equivalents(&self) -> String {
        equivalent_commands(self.view_options(), self.selected.as_deref(), None).join(" ; ")
    }

    pub fn equivalents(&self) -> Vec<String> {
        // The vault panel open on the selected disk: its command is the one to copy.
        if let Some(vault) = self.vault_equivalent() {
            return vec![vault];
        }
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

    pub fn offered(&self, id: &str) -> bool {
        self.entries()
            .iter()
            .any(|(disk, verdict)| disk.id == id && verdict.is_writable())
    }

    pub fn is_internal(&self, id: &str) -> bool {
        self.entries().iter().any(|(disk, verdict)| {
            disk.id == id
                && *verdict
                    == (Eligibility::Offered {
                        warning: Warning::InternalDisk,
                    })
        })
    }

    /// The panel may have changed since the prompt opened.
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
