use std::time::Instant;

use bonk_core::format::table::TableKind;
use bonk_core::write::Phase;

use crate::command::WriteView;
use crate::event::{Event, write_args};
use crate::model::{Effect, Effects, Focus, Message, Model, WRITE_DELAY, WriteState};

impl Model {
    /// Starting, following and closing a write. [`Self::apply`] sends no other message here.
    pub(super) fn apply_write(&mut self, message: Message) -> Effects {
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
        self.network_closed()
            && ["http://", "https://", "ftp://"]
                .iter()
                .any(|scheme| image.starts_with(scheme))
    }

    /// Why a web address is not downloaded now.
    #[must_use]
    pub fn address_refusal(&self) -> &'static str {
        if self.offline {
            self.lang.pick(
                "Offline mode: a web address is not downloaded.",
                "Mode hors ligne : une adresse web n'est pas téléchargée.",
            )
        } else {
            self.lang.pick(
                "Waiting for Tor: a web address is downloaded only through it.",
                "En attente de Tor : une adresse web ne se télécharge que par lui.",
            )
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
                force: internal_confirmed,
                file_name: self.catalog.file_name_of(image.trim()),
                hybrid: self.catalog.hybrid_of(image.trim()),
            },
            self.view_options(),
            self.proxy(),
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
