use super::*;

impl Model {
    /// The catalog menu. [`Self::apply`] sends no other message here.
    pub(super) fn apply_menu(&mut self, message: Message) -> Effects {
        match message {
            Message::RequestImage(id) => return self.request_image(id),
            Message::CatalogLoaded(result) => return self.on_catalog_loaded(result),
            Message::MenuChose(choice) => return self.on_menu_chosen(choice),
            Message::ToggleMenu => {
                self.focus = Some(Focus::Menu);
                self.menu_open = !self.menu_open;
            }
            Message::MenuCategory(category) => {
                self.menu_filter.category = category;
                self.menu_filter.system = None;
                self.menu_index = 0;
            }
            Message::MenuBase(base) => {
                self.menu_filter.base = base;
                self.menu_filter.system = None;
                self.menu_index = 0;
            }
            Message::MenuSystem(system) => {
                self.menu_filter.system = system;
                self.menu_index = 0;
            }
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn on_catalog_loaded(&mut self, result: Result<catalog::Shelf, String>) -> Effects {
        // A read started before the switch may still answer.
        if self.offline {
            return Vec::new();
        }
        self.catalog = match result {
            Ok(shelf) => {
                self.terms = shelf.terms;
                CatalogState::Ready(shelf.systems)
            }
            Err(error) => {
                log::warn!(target: "bonk::gui", reason = error.as_str(); "catalog unavailable");
                // Without a list the manual way is the only one: it shows itself.
                self.advanced = true;
                CatalogState::Failed(error)
            }
        };
        self.fill_requested();
        Vec::new()
    }

    /// Opens the window for one catalog image. The catalog is read at once, without waiting for a disk.
    pub fn request_image(&mut self, id: String) -> Effects {
        if self.offline {
            log::info!(target: "bonk::gui", image = id.as_str(); "link ignored: offline mode");
            return Vec::new();
        }
        self.requested = Some(id);
        if matches!(self.catalog, CatalogState::Ready(_)) {
            self.fill_requested();
            return Vec::new();
        }
        if matches!(self.catalog, CatalogState::Loading) {
            return Vec::new();
        }
        self.catalog = CatalogState::Loading;
        vec![Effect::LoadCatalog]
    }

    /// Only an empty draft: what the user typed or chose is never replaced.
    pub(super) fn fill_requested(&mut self) {
        let Some((_, crate::model::Requested::Ready(choice))) = self.requested() else {
            return;
        };
        let ChoiceSource::Download { url, sha256 } = choice.source.clone() else {
            return;
        };
        if matches!(&self.write, WriteState::Draft { image, .. } if image.is_empty()) {
            self.fill_draft(&ImageSource::Catalog { url, sha256 });
        }
    }

    pub(super) fn on_menu_chosen(&mut self, choice: catalog::Choice) -> Effects {
        if let (WriteState::Draft { .. }, ChoiceSource::Download { url, sha256 }) =
            (&self.write, choice.source)
        {
            self.fill_draft(&ImageSource::Catalog { url, sha256 });
            self.requested = None;
            self.menu_open = false;
            self.focus = Some(Focus::Action);
        }
        Vec::new()
    }

    /// The write panel just opened: the menu must be ready. The catalog is only read here, and reread after a
    /// failure at the next selection.
    pub(crate) fn load_menu(&mut self) -> Effects {
        let needed = matches!(
            self.catalog,
            CatalogState::NotLoaded | CatalogState::Failed(_)
        );
        if self.offline || !needed || !matches!(self.write, WriteState::Draft { .. }) {
            return Vec::new();
        }
        self.catalog = CatalogState::Loading;
        vec![Effect::LoadCatalog]
    }
}
