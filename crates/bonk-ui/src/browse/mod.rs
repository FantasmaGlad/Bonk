mod filter;
mod labels;

pub use filter::{BrowseFilter, Options, options};
pub use labels::{desktop_label, media_label};

use crate::catalog::{CatalogState, ChoiceSource, System};
use crate::model::{Effect, Effects, Focus, Model};
use crate::settings::Screen;

/// The catalog tab: its filters and the system whose variants are shown.
#[derive(Debug, Default)]
pub struct BrowseState {
    pub filter: BrowseFilter,
    /// By name: one page per system.
    pub opened: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
            return vec![Effect::LoadCatalog];
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
                order.extend(
                    system
                        .choices
                        .iter()
                        .filter(|choice| self.browse.filter.keeps_choice(choice))
                        .filter(|choice| matches!(choice.source, ChoiceSource::Download { .. }))
                        .map(|choice| Focus::Variant(choice.id.clone())),
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
