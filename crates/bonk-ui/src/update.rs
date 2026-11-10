use std::time::Instant;

use bonk_core::format::table::TableKind;
use bonk_core::write::Phase;
use bonk_hal::{Disk, Lang};

use crate::catalog::{self, CatalogState, ChoiceSource};
use crate::command::WriteView;
use crate::event::{Event, write_args};
use crate::model::{Effect, Effects, Focus, Message, Model, WRITE_DELAY, WriteState};

impl Model {
    // A flat dispatch, one arm per message: splitting it would make it harder to follow.
    #[allow(clippy::too_many_lines)]
    pub fn apply(&mut self, message: Message) -> Effects {
        match message {
            Message::Refresh => {
                self.focus = Some(Focus::Refresh);
                self.reload()
            }
            Message::Tick => {
                log::trace!(target: "bonk::gui", "tick");
                self.reload()
            }
            Message::Loaded(Ok(disks)) => self.on_disks_loaded(disks),
            Message::Loaded(Err(erreur)) => self.on_detection_failed(erreur),
            Message::ToggleInternal(on) => {
                self.show_internal = on;
                self.after_view_option(Focus::Internal)
            }
            Message::ToggleVirtual(on) => {
                self.allow_virtual = on;
                self.after_view_option(Focus::Virtual)
            }
            Message::ToggleAll(on) => {
                self.show_all = on;
                self.after_view_option(Focus::All)
            }
            Message::Select(id) => {
                self.focus = Some(Focus::Disk(id.clone()));
                self.selected = if self.selected.as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id)
                };
                self.sync_write();
                self.load_menu()
            }
            Message::Nav(nav) => self.navigate(nav),
            Message::Copy => {
                self.focus = Some(Focus::Copy);
                vec![Effect::Copy(self.equivalents().join("\n"))]
            }
            Message::ImageInput(text) => {
                if let WriteState::Draft { image, .. } = &mut self.write {
                    *image = text;
                    self.focus = Some(Focus::Image);
                    self.requested = None;
                }
                Vec::new()
            }
            Message::HashInput(text) => {
                if let WriteState::Draft { hash, .. } = &mut self.write {
                    *hash = text;
                    self.focus = Some(Focus::Hash);
                }
                Vec::new()
            }
            Message::ToggleRaw(on) => {
                if let WriteState::Draft { raw, .. } = &mut self.write {
                    *raw = on;
                    self.focus = Some(Focus::Raw);
                }
                Vec::new()
            }
            Message::ToggleMbr(on) => {
                if let WriteState::Draft { mbr, .. } = &mut self.write {
                    *mbr = on;
                    self.focus = Some(Focus::Mbr);
                }
                Vec::new()
            }
            Message::BrowseImage => {
                self.focus = Some(Focus::Browse);
                vec![Effect::PickImage]
            }
            Message::ImagePicked(Some(path)) => {
                if let WriteState::Draft { image, .. } = &mut self.write {
                    *image = path.display().to_string();
                    // A dropped image must be visible: the menu does not name it.
                    self.advanced = true;
                    self.requested = None;
                }
                Vec::new()
            }
            Message::ImagePicked(None) => Vec::new(),
            Message::TorDetected(address) => {
                self.tor = address;
                Vec::new()
            }
            Message::SwitchLanguage => self.switch_language(),
            Message::ToggleTor(on) => {
                self.use_tor = on && self.tor.is_some();
                self.focus = Some(Focus::Tor);
                Vec::new()
            }
            Message::ToggleMenu => {
                self.focus = Some(Focus::Menu);
                self.menu_open = !self.menu_open;
                Vec::new()
            }
            Message::MenuCategory(category) => {
                self.menu_filter = catalog::MenuFilter {
                    category,
                    system: None,
                };
                self.menu_index = 0;
                Vec::new()
            }
            Message::MenuSystem(system) => {
                self.menu_filter.system = system;
                self.menu_index = 0;
                Vec::new()
            }
            Message::CatalogLoaded(result) => self.on_catalog_loaded(result),
            Message::MenuChose(choice) => self.on_menu_chosen(choice),
            Message::ToggleAdvanced => {
                self.advanced = !self.advanced;
                self.focus = Some(Focus::Advanced);
                Vec::new()
            }
            Message::Clock => Vec::new(),
            Message::StartWrite => self.start_write(false),
            Message::ConfirmInternal => {
                self.confirming_internal = false;
                self.start_write(true)
            }
            Message::CloseInternal => {
                self.confirming_internal = false;
                self.focus = Some(Focus::Action);
                Vec::new()
            }
            Message::CancelWrite => {
                self.focus = Some(Focus::Action);
                if let WriteState::Running { pid: Some(pid), .. } = &self.write {
                    log::info!(target: "bonk::gui", pid = *pid; "cancelling the write");
                    return vec![Effect::CancelWrite(*pid)];
                }
                Vec::new()
            }
            Message::DismissWrite => {
                self.write = WriteState::Idle;
                self.selected = None;
                self.requested = None;
                self.fix_focus();
                Vec::new()
            }
            Message::Write(event) => {
                self.on_write_event(event);
                Vec::new()
            }
        }
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

    /// A display box changed: the list too, hence the panel and the focus.
    fn after_view_option(&mut self, focus: Focus) -> Effects {
        self.focus = Some(focus);
        self.sync_write();
        self.fix_focus();
        Vec::new()
    }

    fn switch_language(&mut self) -> Effects {
        self.lang = match self.lang {
            Lang::En => Lang::Fr,
            Lang::Fr => Lang::En,
        };
        self.focus = Some(Focus::Language);
        vec![Effect::SaveLanguage(self.lang)]
    }

    fn on_catalog_loaded(&mut self, result: Result<Vec<catalog::System>, String>) -> Effects {
        self.catalog = match result {
            Ok(systems) => CatalogState::Ready(systems),
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
            self.fill_draft(url, sha256);
        }
    }

    /// The catalog fixes everything: Bonk analyses the image and picks the mode itself.
    fn fill_draft(&mut self, url: String, sha256: String) {
        if let WriteState::Draft {
            image,
            hash,
            raw,
            mbr,
            ..
        } = &mut self.write
        {
            *image = url;
            *hash = sha256;
            *raw = false;
            *mbr = TableKind::WINDOWS_DEFAULT == TableKind::Mbr;
        }
    }

    fn on_menu_chosen(&mut self, choice: catalog::Choice) -> Effects {
        if let (WriteState::Draft { .. }, ChoiceSource::Download { url, sha256 }) =
            (&self.write, choice.source)
        {
            self.fill_draft(url, sha256);
            self.requested = None;
            self.menu_open = false;
            self.focus = Some(Focus::Action);
        }
        Vec::new()
    }

    /// The write panel just opened: the menu must be ready. The catalog is only read here, and reread after a
    /// failure at the next selection.
    fn load_menu(&mut self) -> Effects {
        let needed = matches!(
            self.catalog,
            CatalogState::NotLoaded | CatalogState::Failed(_)
        );
        if !needed || !matches!(self.write, WriteState::Draft { .. }) {
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
            }
            (_, None) => self.write = WriteState::Idle,
        }
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
        if image.trim().is_empty() || opened.elapsed() < WRITE_DELAY || !self.offered(disk_id) {
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
