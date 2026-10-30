//! Effet des messages sur l'état de la fenêtre, et suivi du processus d'écriture.

use std::path::PathBuf;
use std::time::Instant;

use iced::Task;

use crate::transfer;

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
                // Un disque débranché ne reste pas sélectionné.
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|id| !disks.iter().any(|d| &d.id == id))
                {
                    self.selected = None;
                }
                self.disks = disks;
                self.sync_write();
                self.fix_focus();
                Task::none()
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
                Task::none()
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
                self.sync_write();
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

    /// Fait suivre le panneau d'écriture à la sélection : il s'ouvre sur un disque proposé, se referme
    /// sinon, et ne bouge jamais pendant qu'une écriture tourne.
    pub(super) fn sync_write(&mut self) {
        if matches!(self.write, WriteState::Running { .. }) {
            return;
        }
        let wanted = self.selected.clone().filter(|id| self.offered(id));
        match (&self.write, wanted) {
            (current, Some(id)) if current.disk_id() == Some(id.as_str()) => {}
            (_, Some(id)) => {
                self.write = WriteState::Draft {
                    disk_id: id,
                    image: String::new(),
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
            image.trim(),
            self.view_options(),
            internal_confirmed,
        );
        log::info!(target: "bonk::gui", disk = disk_id.as_str(); "starting the write");
        let receiver = transfer::spawn(&self.program(), &args);
        self.write = WriteState::Running {
            disk_id: disk_id.clone(),
            pid: None,
            phase: "write".to_owned(),
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
