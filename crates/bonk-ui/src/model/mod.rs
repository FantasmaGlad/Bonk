use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_core::write::Phase;
use bonk_hal::{Disk, Lang};

use crate::command::ViewOptions;
use crate::event::{Event, Outcome};

mod events;
mod queries;

pub use events::*;

pub const REFRESH_EVERY: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Enumerate,
    LoadCatalog {
        proxy: Option<String>,
    },
    DetectTor,
    Copy(String),
    PickImage,
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
    RunVault(Vec<String>),
    RunDownload(Vec<String>),
    PickVaultIso,
    ListKept,
    RemoveKept(String),
    InspectImage(String),
    /// Opened in the user's browser; only fixed addresses of Bonk itself.
    OpenLink(&'static str),
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
    Select(String),
    ToggleDetails,
    Nav(Nav),
    Copy,
    ImageInput(String),
    HashInput(String),
    ToggleRaw(bool),
    TorDetected(Option<String>),
    ToggleTor(bool),
    OpenTorProject,
    Browse(crate::browse::BrowseMessage),
    ToggleMbr(bool),
    BrowseImage,
    /// `None`: closed without a choice.
    ImagePicked(Option<PathBuf>),
    Inspected(String, Option<serde_json::Value>),
    ToggleMenu,
    MenuCategory(Option<String>),
    MenuBase(Option<String>),
    MenuSystem(Option<String>),
    MenuSearch(String),
    MenuArch(Option<String>),
    CatalogLoaded(Result<crate::catalog::Shelf, String>),
    MenuChose(crate::catalog::Choice),
    ToggleAdvanced,
    Clock,
    StartWrite,
    CancelWrite,
    DismissWrite,
    ConfirmInternal,
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
    Settings,
    CloseSettings,
    Library,
    CloseLibrary,
    Catalog,
    CloseCatalog,
    CatalogSearch,
    System(String),
    CloseSystem,
    Variant(String),
    Download(String),
    DownloadAction,
    KeptWrite(String),
    KeptRemove(String),
    Language(crate::settings::LangChoice),
    Theme(crate::settings::ThemeChoice),
    Internal,
    Virtual,
    All,
    Offline,
    IncognitoAtStart,
    Notify,
    Disk(String),
    Details,
    Image,
    Browse,
    Hash,
    Menu,
    Advanced,
    Tor,
    Raw,
    Mbr,
    Action,
    Copy,
}

pub const WRITE_DELAY: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub enum WriteState {
    #[default]
    Idle,
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
    pub catalog_saved: bool,
    pub offline: bool,
    pub terms: crate::terms::Terms,
    pub menu_filter: crate::catalog::MenuFilter,
    /// A local Tor, offered by the incognito button; never used without it.
    pub tor: Option<String>,
    pub incognito_at_start: bool,
    pub notify: bool,
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
    pub inspection: Option<crate::inspect::Inspection>,
    pub vault: crate::vault::VaultState,
    pub library: crate::library::LibraryState,
    pub browse: crate::browse::BrowseState,
    pub screen: crate::settings::Screen,
    pub lang_choice: crate::settings::LangChoice,
    pub theme_choice: crate::settings::ThemeChoice,
    pub dark: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requested<'a> {
    Waiting,
    Ready(&'a crate::catalog::Choice),
    NotWritable(&'a crate::catalog::Choice),
    Unknown,
    CatalogFailed,
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
        let model = Self {
            lang,
            loading: true,
            auto_select: true,
            show_internal: initial.show_internal,
            allow_virtual: initial.allow_virtual,
            show_all: initial.show_all,
            offline: initial.offline,
            advanced: initial.offline,
            incognito_at_start: initial.incognito,
            notify: initial.notify,
            use_tor: initial.incognito && !initial.offline,
            ..Self::default()
        };
        let mut effects = vec![Effect::Enumerate];
        effects.extend(model.watch_tor());
        (model, effects)
    }

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
}
