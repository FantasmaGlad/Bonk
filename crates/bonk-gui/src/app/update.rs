//! Effet des messages sur l'état de la fenêtre, et suivi du processus d'écriture.

use std::path::PathBuf;
use std::time::Instant;

use iced::Task;

use bonk_core::write::Phase;

use crate::command::WriteView;
use crate::transfer;

use bonk_hal::Lang;

use super::catalog::{self, CatalogState, ChoiceSource};
use super::{App, Focus, Message, WRITE_DELAY, WriteState, load};

impl App {
    pub(super) fn apply(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => {
                self.focus = Some(Focus::Refresh);
                self.reload()
            }
            Message::Tick => {
                log::trace!(target: "bonk::gui", "tick");
                self.reload()
            }
            Message::Loaded(Ok(disks)) => {
                log::debug!(target: "bonk::gui", disks = disks.len(); "enumeration result received");
                self.loading = false;
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
            Message::Loaded(Err(erreur)) => {
                log::warn!(target: "bonk::gui", reason = erreur.as_str(); "enumeration failed");
                self.loading = false;
                // Jamais de liste périmée présentée comme valable après un échec.
                self.disks.clear();
                self.selected = None;
                self.error = Some(erreur);
                self.sync_write();
                self.fix_focus();
                Task::none()
            }
            Message::ToggleInternal(on) => {
                self.show_internal = on;
                self.focus = Some(Focus::Internal);
                self.sync_write();
                self.fix_focus();
                Task::none()
            }
            Message::ToggleVirtual(on) => {
                self.allow_virtual = on;
                self.focus = Some(Focus::Virtual);
                self.sync_write();
                self.fix_focus();
                Task::none()
            }
            Message::ToggleAll(on) => {
                self.show_all = on;
                self.focus = Some(Focus::All);
                self.sync_write();
                self.fix_focus();
                Task::none()
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
                iced::clipboard::write(self.equivalents().join("\n"))
            }
            Message::ImageInput(text) => {
                if let WriteState::Draft { image, .. } = &mut self.write {
                    *image = text;
                    self.focus = Some(Focus::Image);
                }
                Task::none()
            }
            Message::HashInput(text) => {
                if let WriteState::Draft { hash, .. } = &mut self.write {
                    *hash = text;
                    self.focus = Some(Focus::Hash);
                }
                Task::none()
            }
            Message::ToggleRaw(on) => {
                if let WriteState::Draft { raw, .. } = &mut self.write {
                    *raw = on;
                    self.focus = Some(Focus::Raw);
                }
                Task::none()
            }
            Message::ToggleMbr(on) => {
                if let WriteState::Draft { mbr, .. } = &mut self.write {
                    *mbr = on;
                    self.focus = Some(Focus::Mbr);
                }
                Task::none()
            }
            Message::BrowseImage => {
                self.focus = Some(Focus::Browse);
                Task::perform(pick_image(self.lang), Message::ImagePicked)
            }
            Message::ImagePicked(Some(path)) => {
                if let WriteState::Draft { image, .. } = &mut self.write {
                    *image = path.display().to_string();
                    // Une image déposée doit se voir : le menu ne la nomme pas.
                    self.advanced = true;
                }
                Task::none()
            }
            Message::ImagePicked(None) => Task::none(),
            Message::TorDetected(address) => {
                self.tor = address;
                Task::none()
            }
            Message::SwitchLanguage => {
                self.lang = match self.lang {
                    Lang::En => Lang::Fr,
                    Lang::Fr => Lang::En,
                };
                self.focus = Some(Focus::Language);
                let program = self.program();
                let value = match self.lang {
                    Lang::En => "en",
                    Lang::Fr => "fr",
                };
                // Le réglage passe par la ligne de commande, seule propriétaire du fichier de configuration.
                Task::perform(async move { save_language(&program, value) }, |()| {
                    Message::Refresh
                })
                .discard()
            }
            Message::ToggleTor(on) => {
                self.use_tor = on && self.tor.is_some();
                self.focus = Some(Focus::Tor);
                Task::none()
            }
            Message::ToggleMenu => {
                self.focus = Some(Focus::Menu);
                self.menu_open = !self.menu_open;
                Task::none()
            }
            Message::MenuCategory(category) => {
                self.menu_filter = catalog::MenuFilter {
                    category,
                    system: None,
                };
                self.menu_index = 0;
                Task::none()
            }
            Message::MenuSystem(system) => {
                self.menu_filter.system = system;
                self.menu_index = 0;
                Task::none()
            }
            Message::CatalogLoaded(result) => {
                self.catalog = match result {
                    Ok(systems) => CatalogState::Ready(systems),
                    Err(error) => {
                        log::warn!(target: "bonk::gui", reason = error.as_str(); "catalog unavailable");
                        // Sans liste, la voie manuelle est la seule : elle se montre d'elle-même.
                        self.advanced = true;
                        CatalogState::Failed(error)
                    }
                };
                Task::none()
            }
            Message::MenuChose(choice) => {
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
                Task::none()
            }
            Message::ToggleAdvanced => {
                self.advanced = !self.advanced;
                self.focus = Some(Focus::Advanced);
                Task::none()
            }
            Message::Clock => Task::none(),
            Message::StartWrite => self.start_write(false),
            Message::ConfirmInternal => {
                self.confirming_internal = false;
                self.start_write(true)
            }
            Message::CloseInternal => {
                self.confirming_internal = false;
                self.focus = Some(Focus::Action);
                Task::none()
            }
            Message::CancelWrite => {
                self.focus = Some(Focus::Action);
                if let WriteState::Running { pid: Some(pid), .. } = &self.write {
                    log::info!(target: "bonk::gui", pid = *pid; "cancelling the write");
                    transfer::cancel(*pid);
                }
                Task::none()
            }
            Message::DismissWrite => {
                self.write = WriteState::Idle;
                self.selected = None;
                self.fix_focus();
                Task::none()
            }
            Message::Write(event) => {
                self.on_write_event(event);
                Task::none()
            }
        }
    }

    /// Lance une énumération, une seule à la fois : le minuteur ne les empile pas si l'une est lente.
    pub(super) fn reload(&mut self) -> Task<Message> {
        if self.loading {
            Task::none()
        } else {
            self.loading = true;
            load()
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

    /// Le panneau d'écriture vient de s'ouvrir : le menu doit être prêt. Le catalogue n'est lu que là, et relu
    /// après un échec à la prochaine sélection.
    fn load_menu(&mut self) -> Task<Message> {
        let needed = matches!(
            self.catalog,
            CatalogState::NotLoaded | CatalogState::Failed(_)
        );
        if !needed || !matches!(self.write, WriteState::Draft { .. }) {
            return Task::none();
        }
        self.catalog = CatalogState::Loading;
        let program = self.program();
        Task::batch([
            Task::perform(
                async move { catalog::read(program) },
                Message::CatalogLoaded,
            ),
            Task::perform(async { bonk_catalog::tor::detect() }, Message::TorDetected),
        ])
    }

    /// Fait suivre le panneau d'écriture à la sélection : il s'ouvre sur un disque proposé, se referme
    /// sinon. Une écriture en cours ou son résultat ne bougent pas : le disque a pu partir, et l'on doit
    /// encore pouvoir annuler ou lire le code d'erreur.
    pub(super) fn sync_write(&mut self) {
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

    pub(super) fn program(&self) -> PathBuf {
        self.program
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| PathBuf::from("bonk"))
    }

    /// Lance `bonk write` dans un processus séparé, une fois le délai de réflexion écoulé.
    pub(super) fn start_write(&mut self, internal_confirmed: bool) -> Task<Message> {
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
            return Task::none();
        };
        if image.trim().is_empty() || opened.elapsed() < WRITE_DELAY || !self.offered(disk_id) {
            return Task::none();
        }
        if self.is_internal(disk_id) && !internal_confirmed {
            self.confirming_internal = true;
            return Task::none();
        }
        let args = transfer::write_args(
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
        let receiver = transfer::spawn(&self.program(), &args);
        self.write = WriteState::Running {
            disk_id: disk_id.clone(),
            pid: None,
            phase: Phase::Write,
            done: 0,
            total: None,
            rate: 0.0,
        };
        Task::run(receiver, Message::Write)
    }

    pub(super) fn on_write_event(&mut self, event: transfer::Event) {
        match (event, &mut self.write) {
            (transfer::Event::Started(new), WriteState::Running { pid, .. }) => *pid = Some(new),
            (
                transfer::Event::Progress {
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
            (transfer::Event::Finished(outcome), WriteState::Running { disk_id, .. }) => {
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

/// La fenêtre change de langue même si l'enregistrement échoue : seul le prochain lancement l'oublie.
fn save_language(program: &std::path::Path, value: &str) {
    let mut command = std::process::Command::new(program);
    bonk_hal::console::hide_child_console(&mut command);
    let saved = command
        .args(["config", "set", "lang", value])
        .stdin(std::process::Stdio::null())
        .output();
    if !saved.as_ref().is_ok_and(|output| output.status.success()) {
        log::warn!(target: "bonk::gui", "language choice not saved");
    }
}

/// Le sélecteur de fichier du système, limité aux formats que Bonk écrit.
async fn pick_image(lang: Lang) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title(lang.pick("Choose the image to write", "Choisir l'image à écrire"))
        .add_filter(
            lang.pick("Disk images", "Images de disque"),
            &["iso", "img", "raw", "gz", "xz", "zst"],
        )
        .add_filter(lang.pick("All files", "Tous les fichiers"), &["*"])
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}
