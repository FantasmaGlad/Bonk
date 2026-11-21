use crate::model::{Effect, Effects, Focus, Message, Model};

mod disks;
mod draft;
mod menu;
mod write;

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
            Message::Browse(message) => self.apply_browse(message),
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
            | Message::ToggleAdvanced) => self.apply_draft(message),
            message @ (Message::TorDetected(_)
            | Message::ToggleTor(_)
            | Message::OpenTorProject) => self.apply_incognito(message),
            message @ (Message::RequestImage(_)
            | Message::ToggleMenu
            | Message::MenuCategory(_)
            | Message::MenuBase(_)
            | Message::MenuSystem(_)
            | Message::MenuSearch(_)
            | Message::MenuArch(_)
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
    pub(super) fn apply_view(&mut self, message: Message) -> Effects {
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

    /// One at a time: the timer does not stack them if one is slow.
    pub fn reload(&mut self) -> Effects {
        let mut effects: Effects = self.watch_tor().into_iter().collect();
        if !self.loading {
            self.loading = true;
            effects.push(Effect::Enumerate);
        }
        effects
    }

    pub(super) fn after_view_option(
        &mut self,
        focus: Focus,
        key: &'static str,
        on: bool,
    ) -> Effects {
        self.focus = Some(focus);
        self.sync_write();
        self.fix_focus();
        vec![Effect::SaveSetting {
            key,
            value: if on { "true" } else { "false" },
        }]
    }
}
