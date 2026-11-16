use std::path::PathBuf;
use std::time::{Duration, Instant};

use bonk_core::write::Phase;
use bonk_hal::{Disk, Lang};

use crate::command::ViewOptions;
use crate::event::{Event, Outcome};

mod events;
mod queries;

pub use events::*;

/// A plugged disk appears without a user action.
pub const REFRESH_EVERY: Duration = Duration::from_secs(2);

/// What the interface must do after a message; the result comes back as another message.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Enumerate,
    /// `bonk catalog list`, through Tor when the interface is incognito.
    LoadCatalog {
        proxy: Option<String>,
    },
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
    /// `bonk catalog download` arguments, run as `bonk write` is.
    RunDownload(Vec<String>),
    PickVaultIso,
    /// `bonk cache list --json`, answered by [`crate::library::LibraryMessage::Listed`].
    ListKept,
    /// `bonk cache clean <SHA256>`, answered by [`crate::library::LibraryMessage::Removed`].
    RemoveKept(String),
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
    /// Selects or deselects a disk, by id.
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
    Settings,
    CloseSettings,
    Library,
    CloseLibrary,
    /// The catalog tab button, its back button, its search field.
    Catalog,
    CloseCatalog,
    CatalogSearch,
    /// A system card of the catalog tab, by name.
    System(String),
    CloseSystem,
    /// A writable variant on a system page, by catalog id.
    Variant(String),
    /// Its download button.
    Download(String),
    /// Cancels the running download, or closes its result.
    DownloadAction,
    /// Write or remove the kept image with this hash.
    KeptWrite(String),
    KeptRemove(String),
    Language(crate::settings::LangChoice),
    Theme(crate::settings::ThemeChoice),
    Internal,
    Virtual,
    All,
    Offline,
    IncognitoAtStart,
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
    /// The network or Tor did not answer: the catalog shown is the last verified copy.
    pub catalog_saved: bool,
    /// Never reach the network: the catalog is neither read nor shown, and a web address is refused.
    pub offline: bool,
    /// Labels of the usage categories and bases, from the catalog once read.
    pub terms: crate::terms::Terms,
    pub menu_filter: crate::catalog::MenuFilter,
    /// A local Tor, offered by the incognito button; never used without it.
    pub tor: Option<String>,
    /// Kept setting: start in incognito, and wait for Tor rather than reach the network without it.
    pub incognito_at_start: bool,
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
    pub browse: crate::browse::BrowseState,
    pub screen: crate::settings::Screen,
    pub lang_choice: crate::settings::LangChoice,
    pub theme_choice: crate::settings::ThemeChoice,
    /// The window's theme family, resolved once before it opens.
    pub dark: bool,
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
            use_tor: initial.incognito && !initial.offline,
            ..Self::default()
        };
        let mut effects = vec![Effect::Enumerate];
        effects.extend(model.watch_tor());
        (model, effects)
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
}
