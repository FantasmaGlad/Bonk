//! Effet des messages sur l'état de la fenêtre, et suivi du processus d'écriture.

use std::time::Instant;

use bonk_core::write::Phase;
use bonk_hal::{Disk, Lang};

use crate::catalog::{self, CatalogState, ChoiceSource};
use crate::command::WriteView;
use crate::event::{Event, write_args};
use crate::model::{Effect, Effects, Focus, Message, Model, WRITE_DELAY, WriteState};

impl Model {
    // Un aiguillage plat, un bras par message : le découper le rendrait plus difficile à parcourir.
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
                    // Une image déposée doit se voir : le menu ne la nomme pas.
                    self.advanced = true;
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
                self.fix_focus();
                Vec::new()
            }
            Message::Write(event) => {
                self.on_write_event(event);
                Vec::new()
            }
        }
    }

    /// Lance une énumération, une seule à la fois : le minuteur ne les empile pas si l'une est lente.
    pub fn reload(&mut self) -> Effects {
        if self.loading {
            Vec::new()
        } else {
            self.loading = true;
            vec![Effect::Enumerate]
        }
    }

    /// Un seul disque proposé, et aucun choix encore : le sélectionner épargne un clic. Une fois par disque, pour
    /// qu'une désélection tienne malgré l'actualisation.
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
        // Jamais de liste périmée présentée comme valable après un échec.
        self.disks.clear();
        self.selected = None;
        self.error = Some(erreur);
        self.sync_write();
        self.fix_focus();
        Vec::new()
    }

    /// Une case d'affichage a changé : la liste aussi, donc le panneau et le focus.
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
                // Sans liste, la voie manuelle est la seule : elle se montre d'elle-même.
                self.advanced = true;
                CatalogState::Failed(error)
            }
        };
        Vec::new()
    }

    fn on_menu_chosen(&mut self, choice: catalog::Choice) -> Effects {
        if let (
            WriteState::Draft {
                image,
                hash,
                raw,
                mbr,
                ..
            },
            ChoiceSource::Download { url, sha256 },
        ) = (&mut self.write, choice.source)
        {
            // Tout est fixé par le catalogue : Bonk analyse l'image et choisit le mode seul.
            *image = url;
            *hash = sha256;
            *raw = false;
            *mbr = false;
            self.menu_open = false;
            self.focus = Some(Focus::Action);
        }
        Vec::new()
    }

    /// Le panneau d'écriture vient de s'ouvrir : le menu doit être prêt. Le catalogue n'est lu que là, et relu
    /// après un échec à la prochaine sélection.
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

    /// Fait suivre le panneau d'écriture à la sélection : il s'ouvre sur un disque proposé, se referme
    /// sinon. Une écriture en cours ou son résultat ne bougent pas : le disque a pu partir, et l'on doit
    /// encore pouvoir annuler ou lire le code d'erreur.
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
                    mbr: false,
                    opened: Instant::now(),
                };
            }
            (_, None) => self.write = WriteState::Idle,
        }
    }

    /// Lance `bonk write` dans un processus séparé, une fois le délai de réflexion écoulé.
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
