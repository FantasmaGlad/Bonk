mod download;
mod filter;
mod labels;
mod text;

pub use download::{DownloadState, Step, download_args};
pub use filter::{BrowseFilter, Options, options};
pub use labels::{desktop_label, media_label};
pub use text::{variant_facts, writable_count};

use crate::catalog::{CatalogState, ChoiceSource, System};
use crate::model::{Effects, Focus, Model};
use crate::settings::Screen;

/// The catalog tab: its filters and the system whose variants are shown.
#[derive(Debug, Default)]
pub struct BrowseState {
    pub filter: BrowseFilter,
    /// By name: one page per system.
    pub opened: Option<String>,
    pub download: Option<DownloadState>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BrowseMessage {
    Open,
    Close,
    Search(String),
    Uses(Option<String>),
    Base(Option<String>),
    Desktop(Option<String>),
    Media(Option<String>),
    OpenSystem(String),
    CloseSystem,
    /// Back to the drives with this catalog image asked for.
    Write(String),
    /// Kept in the library without being written.
    Download(String),
    CancelDownload,
    DownloadEvent(crate::event::Event),
    DismissDownload,
}

impl Model {
    /// Shown while the catalog tab is open; `None` on the grid.
    #[must_use]
    pub fn opened_system(&self) -> Option<&System> {
        let CatalogState::Ready(systems) = &self.catalog else {
            return None;
        };
        let name = self.browse.opened.as_ref()?;
        systems.iter().find(|system| &system.name == name)
    }

    #[must_use]
    pub fn browsed(&self) -> Vec<&System> {
        match &self.catalog {
            CatalogState::Ready(systems) => self.browse.filter.apply(systems),
            _ => Vec::new(),
        }
    }

    pub fn apply_browse(&mut self, message: BrowseMessage) -> Effects {
        let filter = &mut self.browse.filter;
        match message {
            BrowseMessage::Open => return self.open_browse(),
            BrowseMessage::Close => {
                self.screen = Screen::Main;
                self.focus = Some(Focus::Catalog);
            }
            BrowseMessage::Search(text) => filter.search = text,
            BrowseMessage::Uses(id) => filter.uses = id,
            BrowseMessage::Base(id) => filter.base = id,
            BrowseMessage::Desktop(id) => filter.desktop = id,
            BrowseMessage::Media(id) => filter.media = id,
            BrowseMessage::OpenSystem(name) => {
                self.browse.opened = Some(name);
                self.focus = Some(Focus::CloseSystem);
            }
            BrowseMessage::CloseSystem => {
                self.focus = self.browse.opened.take().map(Focus::System);
            }
            BrowseMessage::Write(id) => return self.write_from_catalog(id),
            BrowseMessage::Download(id) => return self.start_download(id),
            BrowseMessage::CancelDownload => return self.cancel_download(),
            BrowseMessage::DownloadEvent(event) => return self.download_event(event),
            BrowseMessage::DismissDownload => {
                if !self
                    .browse
                    .download
                    .as_ref()
                    .is_some_and(DownloadState::running)
                {
                    self.browse.download = None;
                }
            }
        }
        Vec::new()
    }

    fn open_browse(&mut self) -> Effects {
        if self.offline {
            return Vec::new();
        }
        self.screen = Screen::Catalog;
        self.focus = Some(Focus::CloseCatalog);
        if matches!(
            self.catalog,
            CatalogState::NotLoaded | CatalogState::Failed(_)
        ) {
            self.catalog = CatalogState::Loading;
            return vec![self.load_catalog()];
        }
        Vec::new()
    }

    /// The image waits for a drive as a `bonk://` link does: the user still picks the drive and confirms.
    fn write_from_catalog(&mut self, id: String) -> Effects {
        let writable = self.opened_system().is_some_and(|system| {
            system.choices.iter().any(|choice| {
                choice.id == id && matches!(choice.source, ChoiceSource::Download { .. })
            })
        });
        if !writable {
            return Vec::new();
        }
        self.screen = Screen::Main;
        self.browse.opened = None;
        self.focus = None;
        self.request_image(id)
    }

    #[must_use]
    pub fn catalog_focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::CloseCatalog];
        match self.opened_system() {
            Some(system) => {
                order.push(Focus::CloseSystem);
                if self.browse.download.is_some() {
                    order.push(Focus::DownloadAction);
                }
                order.extend(
                    system
                        .choices
                        .iter()
                        .filter(|choice| self.browse.filter.keeps_choice(choice))
                        .filter(|choice| matches!(choice.source, ChoiceSource::Download { .. }))
                        .flat_map(|choice| {
                            [
                                Focus::Variant(choice.id.clone()),
                                Focus::Download(choice.id.clone()),
                            ]
                        }),
                );
            }
            None => {
                order.push(Focus::CatalogSearch);
                order.extend(
                    self.browsed()
                        .iter()
                        .map(|system| Focus::System(system.name.clone())),
                );
            }
        }
        order
    }
}

#[cfg(test)]
mod tests;
