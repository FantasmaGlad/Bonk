use std::time::Instant;

use bonk_core::format::table::TableKind;
use bonk_core::write::Phase;
use bonk_hal::Disk;

use crate::catalog::{self, CatalogState, ChoiceSource};
use crate::command::WriteView;
use crate::event::{Event, write_args};
use crate::model::{Effect, Effects, Focus, Message, Model, WRITE_DELAY, WriteState};
use crate::source::ImageSource;

impl Model {
    pub fn apply(&mut self, message: Message) -> Effects {
        match message {
            Message::Tick => {
                log::trace!(target: "bonk::gui", "tick");
                self.reload()
            }
            Message::Loaded(Ok(disks)) => self.on_disks_loaded(disks),
            Message::Loaded(Err(erreur)) => self.on_detection_failed(erreur),
            Message::Nav(nav) => self.navigate(nav),
            Message::Copy => {
                self.focus = Some(Focus::Copy);
                vec![Effect::Copy(self.equivalents().join("\n"))]
            }
            Message::Settings(message) => self.apply_settings(message),
            Message::Vault(message) => self.apply_vault(message),
            Message::Library(message) => self.apply_library(message),
            message @ (Message::ToggleInternal(_)
            | Message::ToggleVirtual(_)
            | Message::ToggleAll(_)
            | Message::ToggleDetails
            | Message::Select(_)) => self.apply_view(message),
            message @ (Message::ImageInput(_)
            | Message::HashInput(_)
            | Message::ToggleRaw(_)
            | Message::ToggleMbr(_)
            | Message::BrowseImage
            | Message::ImagePicked(_)
            | Message::TorDetected(_)
            | Message::ToggleTor(_)
            | Message::ToggleAdvanced) => self.apply_draft(message),
            message @ (Message::RequestImage(_)
            | Message::ToggleMenu
            | Message::MenuCategory(_)
            | Message::MenuBase(_)
            | Message::MenuSystem(_)
            | Message::CatalogLoaded(_)
            | Message::MenuChose(_)) => self.apply_menu(message),
            message @ (Message::Clock
            | Message::StartWrite
            | Message::ConfirmInternal
            | Message::CloseInternal
            | Message::CancelWrite
            | Message::DismissWrite
            | Message::Write(_)) => self.apply_write(message),
        }
    }

    /// Which disks show, and which one is chosen. [`Self::apply`] sends no other message here.
    fn apply_view(&mut self, message: Message) -> Effects {
        match message {
            Message::ToggleInternal(on) => {
                self.show_internal = on;
                self.after_view_option(Focus::Internal, "view.show_internal", on)
            }
            Message::ToggleVirtual(on) => {
                self.allow_virtual = on;
                self.after_view_option(Focus::Virtual, "view.allow_virtual", on)
            }
            Message::ToggleAll(on) => {
                self.show_all = on;
                self.after_view_option(Focus::All, "view.show_all", on)
            }
            Message::ToggleDetails => {
                self.details = !self.details;
                self.focus = Some(Focus::Details);
                Vec::new()
            }
            Message::Select(id) => {
                self.details = false;
                self.focus = Some(Focus::Disk(id.clone()));
                self.selected = if self.selected.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id)
                };
                self.sync_write();
                self.load_menu()
            }
            _ => Vec::new(),
        }
    }

    /// The fields of the write panel. [`Self::apply`] sends no other message here.
    fn apply_draft(&mut self, message: Message) -> Effects {
        let focus = match &message {
            Message::ImageInput(_) => Focus::Image,
            Message::HashInput(_) => Focus::Hash,
            Message::ToggleRaw(_) => Focus::Raw,
            Message::ToggleMbr(_) => Focus::Mbr,
            Message::BrowseImage => Focus::Browse,
            Message::ToggleTor(_) => Focus::Tor,
            Message::ToggleAdvanced => Focus::Advanced,
            Message::ImagePicked(path) => return self.on_image_picked(path.clone()),
            Message::TorDetected(address) => {
                self.tor.clone_from(address);
                return Vec::new();
            }
            _ => return Vec::new(),
        };
        self.focus = Some(focus);
        match (message, &mut self.write) {
            (Message::ImageInput(text), WriteState::Draft { image, .. }) => {
                *image = text;
                self.requested = None;
            }
            (Message::HashInput(text), WriteState::Draft { hash, .. }) => *hash = text,
            (Message::ToggleRaw(on), WriteState::Draft { raw, .. }) => *raw = on,
            (Message::ToggleMbr(on), WriteState::Draft { mbr, .. }) => *mbr = on,
            (Message::BrowseImage, _) => return vec![Effect::PickImage],
            (Message::ToggleTor(on), _) => self.use_tor = on && self.tor.is_some(),
            (Message::ToggleAdvanced, _) => self.advanced = !self.advanced,
            _ => {}
        }
        Vec::new()
    }

    fn on_image_picked(&mut self, path: Option<std::path::PathBuf>) -> Effects {
        if let Some(path) = path
            && matches!(self.write, WriteState::Draft { .. })
        {
            self.fill_draft(&ImageSource::File(path));
            self.requested = None;
        }
        Vec::new()
    }

    /// The catalog menu. [`Self::apply`] sends no other message here.
    fn apply_menu(&mut self, message: Message) -> Effects {
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

    /// Starting, following and closing a write. [`Self::apply`] sends no other message here.
    fn apply_write(&mut self, message: Message) -> Effects {
        match message {
            Message::StartWrite => return self.start_write(false),
            Message::ConfirmInternal => {
                self.confirming_internal = false;
                return self.start_write(true);
            }
            Message::CloseInternal => {
                self.confirming_internal = false;
                self.focus = Some(Focus::Action);
            }
            Message::CancelWrite => {
                self.focus = Some(Focus::Action);
                if let WriteState::Running { pid: Some(pid), .. } = &self.write {
                    log::info!(target: "bonk::gui", pid = *pid; "cancelling the write");
                    return vec![Effect::CancelWrite(*pid)];
                }
            }
            Message::DismissWrite => {
                self.write = WriteState::Idle;
                self.selected = None;
                self.requested = None;
                self.fix_focus();
            }
            Message::Write(event) => self.on_write_event(event),
            _ => {}
        }
        Vec::new()
    }

    /// One at a time: the timer does not stack them if one is slow.
    pub fn reload(&mut self) -> Effects {
        if self.loading {
            Vec::new()
        } else {
            self.loading = true;
            vec![Effect::Enumerate]
        }
    }

    /// A single offered disk and no choice yet: selecting it saves a click. Once per disk, so a deselection
    /// holds despite the refresh.
    fn select_only_disk(&mut self) {
        if !self.auto_select || self.selected.is_some() {
            return;
        }
        let offered: Vec<String> = self
            .entries()
            .iter()
            .filter(|(_, verdict)| verdict.is_writable())
            .map(|(disk, _)| disk.id.clone())
            .collect();
        if let [only] = offered.as_slice()
            && self.auto_selected.as_ref() != Some(only)
        {
            self.auto_selected = Some(only.clone());
            self.selected = Some(only.clone());
        }
    }

    fn on_disks_loaded(&mut self, disks: Vec<Disk>) -> Effects {
        log::debug!(target: "bonk::gui", disks = disks.len(); "enumeration result received");
        self.loading = false;
        self.answered = true;
        self.error = None;
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !disks.iter().any(|d| &d.id == id))
        {
            self.selected = None;
        }
        self.disks = disks;
        self.select_only_disk();
        self.sync_write();
        self.fix_focus();
        self.load_menu()
    }

    fn on_detection_failed(&mut self, erreur: String) -> Effects {
        log::warn!(target: "bonk::gui", reason = erreur.as_str(); "enumeration failed");
        self.loading = false;
        // Never a stale list shown as valid after a failure.
        self.disks.clear();
        self.selected = None;
        self.error = Some(erreur);
        self.sync_write();
        self.fix_focus();
        Vec::new()
    }

    /// A display box changed: the list too, hence the panel and the focus. Kept for the next start.
    fn after_view_option(&mut self, focus: Focus, key: &'static str, on: bool) -> Effects {
        self.focus = Some(focus);
        self.sync_write();
        self.fix_focus();
        vec![Effect::SaveSetting {
            key,
            value: if on { "true" } else { "false" },
        }]
    }

    fn on_catalog_loaded(&mut self, result: Result<catalog::Shelf, String>) -> Effects {
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
        vec![Effect::LoadCatalog, Effect::DetectTor]
    }

    /// Only an empty draft: what the user typed or chose is never replaced.
    fn fill_requested(&mut self) {
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

    fn on_menu_chosen(&mut self, choice: catalog::Choice) -> Effects {
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
        vec![Effect::LoadCatalog, Effect::DetectTor]
    }

    /// Opens the write panel on an offered disk, closes it otherwise. A running write or its result does not
    /// move: the disk may be gone, and it must still be possible to cancel or read the error code.
    pub fn sync_write(&mut self) {
        if matches!(
            self.write,
            WriteState::Running { .. } | WriteState::Finished { .. }
        ) {
            return;
        }
        let wanted = self.selected.clone().filter(|id| self.offered(id));
        match (&self.write, wanted) {
            (current, Some(id)) if current.disk_id() == Some(id.as_str()) => {}
            (_, Some(id)) => {
                self.write = WriteState::Draft {
                    disk_id: id,
                    image: String::new(),
                    hash: String::new(),
                    raw: false,
                    mbr: TableKind::WINDOWS_DEFAULT == TableKind::Mbr,
                    opened: Instant::now(),
                };
                self.fill_requested();
                if let Some(kept) = self.library.chosen.take() {
                    self.fill_draft(&kept);
                    self.requested = None;
                }
            }
            (_, None) => self.write = WriteState::Idle,
        }
    }

    /// In offline mode an image is a file on this machine: a web address would reach the network.
    #[must_use]
    pub fn refuses_address(&self, image: &str) -> bool {
        let image = image.trim().to_ascii_lowercase();
        self.offline
            && ["http://", "https://", "ftp://"]
                .iter()
                .any(|scheme| image.starts_with(scheme))
    }

    pub fn start_write(&mut self, internal_confirmed: bool) -> Effects {
        self.focus = Some(Focus::Action);
        let WriteState::Draft {
            disk_id,
            image,
            hash,
            raw,
            mbr,
            opened,
        } = &self.write
        else {
            return Vec::new();
        };
        if image.trim().is_empty()
            || opened.elapsed() < WRITE_DELAY
            || !self.offered(disk_id)
            || self.refuses_address(image)
        {
            return Vec::new();
        }
        if self.is_internal(disk_id) && !internal_confirmed {
            self.confirming_internal = true;
            return Vec::new();
        }
        let args = write_args(
            disk_id,
            WriteView {
                image: image.trim(),
                hash,
                raw: *raw,
                mbr: *mbr,
            },
            self.view_options(),
            internal_confirmed,
            self.tor.as_deref().filter(|_| self.use_tor),
        );
        log::info!(target: "bonk::gui", disk = disk_id.as_str(); "starting the write");
        self.write = WriteState::Running {
            disk_id: disk_id.clone(),
            pid: None,
            phase: Phase::Write,
            done: 0,
            total: None,
            rate: 0.0,
        };
        vec![Effect::StartWrite(args)]
    }

    pub fn on_write_event(&mut self, event: Event) {
        match (event, &mut self.write) {
            (Event::Started(new), WriteState::Running { pid, .. }) => *pid = Some(new),
            (
                Event::Progress {
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
            (Event::Finished(outcome), WriteState::Running { disk_id, .. }) => {
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
}
