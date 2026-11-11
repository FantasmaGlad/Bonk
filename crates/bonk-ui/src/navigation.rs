use crate::catalog::{CatalogState, writable};
use crate::model::{Effect, Effects, Focus, Message, Model, Nav, WriteState};
use crate::settings::SettingsMessage;

/// False out of bounds.
pub fn is_disk(order: &[Focus], i: usize) -> bool {
    matches!(order.get(i), Some(Focus::Disk(_)))
}

impl Model {
    /// Tab order: the settings button, then the disks shown.
    pub fn focus_order(&self) -> Vec<Focus> {
        if self.screen == crate::settings::Screen::Settings {
            return self.settings_focus_order();
        }
        let mut order = vec![Focus::Settings];
        if self.error.is_none() {
            for (disk, _) in self.entries() {
                order.push(Focus::Disk(disk.id.clone()));
                if self.selected.as_deref() == Some(disk.id.as_str()) {
                    order.push(Focus::Details);
                }
                if self.write.disk_id() == Some(disk.id.as_str())
                    && matches!(self.write, WriteState::Draft { .. })
                {
                    order.push(Focus::Menu);
                    order.push(Focus::Advanced);
                    if self.advanced {
                        order.push(Focus::Image);
                        order.push(Focus::Browse);
                        order.push(Focus::Hash);
                        order.push(Focus::Tor);
                        order.push(Focus::Raw);
                        order.push(Focus::Mbr);
                    }
                    order.push(Focus::Action);
                }
            }
        }
        // Outside the list: the disk of a running or finished write may be gone.
        if matches!(
            self.write,
            WriteState::Running { .. } | WriteState::Finished { .. }
        ) {
            order.push(Focus::Action);
        }
        order.push(Focus::Copy);
        order
    }

    /// Focus does not stay on a disk gone from the list (unplugged or hidden).
    pub fn fix_focus(&mut self) {
        if self
            .focus
            .as_ref()
            .is_some_and(|focus| !self.focus_order().contains(focus))
        {
            self.focus = None;
        }
    }

    pub fn navigate(&mut self, nav: Nav) -> Effects {
        // In front of the confirmation, the keyboard can only give up: Enter must not erase.
        if self.confirming_internal {
            return if nav == Nav::Escape {
                self.apply(Message::CloseInternal)
            } else {
                Vec::new()
            };
        }
        if nav == Nav::Escape && self.screen == crate::settings::Screen::Settings {
            return self.apply(Message::Settings(SettingsMessage::Close));
        }
        // Menu open: arrows move in it, Escape closes it.
        if self.menu_open {
            match nav {
                Nav::Up | Nav::Down => return self.move_in_menu(nav),
                Nav::Escape => {
                    self.menu_open = false;
                    return Vec::new();
                }
                _ => {}
            }
        }
        let order = self.focus_order();
        let current = self
            .focus
            .as_ref()
            .and_then(|focus| order.iter().position(|item| item == focus));
        let last = order.len() - 1;
        let target = match nav {
            Nav::Next => Some(current.map_or(0, |i| if i == last { 0 } else { i + 1 })),
            Nav::Previous => Some(current.map_or(last, |i| if i == 0 { last } else { i - 1 })),
            // Arrows move between disks; from a control, Down enters the list.
            Nav::Down => match current {
                Some(i) if !matches!(order[i], Focus::Settings | Focus::Copy) => {
                    // Next disk, skipping the current disk's write panel.
                    (i + 1..order.len())
                        .find(|j| is_disk(&order, *j))
                        .or(Some(i))
                }
                _ => order.iter().position(|item| matches!(item, Focus::Disk(_))),
            },
            // Up goes back one disk; from the copy button, to the last disk.
            Nav::Up => match current {
                Some(i)
                    if matches!(
                        order[i],
                        Focus::Disk(_)
                            | Focus::Details
                            | Focus::Menu
                            | Focus::Advanced
                            | Focus::Image
                            | Focus::Browse
                            | Focus::Hash
                            | Focus::Tor
                            | Focus::Raw
                            | Focus::Mbr
                            | Focus::Action
                            | Focus::Copy
                    ) =>
                {
                    // Previous disk; from the panels and the copy button, the nearest disk.
                    (0..i).rev().find(|j| is_disk(&order, *j))
                }
                _ => None,
            },
            Nav::Activate => return self.activate(),
            Nav::Escape => {
                if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.focus = None;
                }
                None
            }
        };
        let Some(index) = target else {
            return Vec::new();
        };
        self.focus = Some(order[index].clone());
        // The list follows the focus: the card's position among the disks (approximate with open details,
        // enough to keep it visible).
        let disk_positions: Vec<usize> = (0..order.len()).filter(|i| is_disk(&order, *i)).collect();
        let rank = disk_positions.iter().position(|i| *i == index);
        if let Some(rank) = rank.filter(|_| disk_positions.len() > 1) {
            let relative = rank as f32 / (disk_positions.len() - 1) as f32;
            return vec![Effect::ScrollList(relative)];
        }
        Vec::new()
    }

    fn move_in_menu(&mut self, nav: Nav) -> Effects {
        let CatalogState::Ready(systems) = &self.catalog else {
            return Vec::new();
        };
        let count = writable(&self.menu_filter.apply(systems)).len();
        if count == 0 {
            return Vec::new();
        }
        self.menu_index = match nav {
            Nav::Up => self.menu_index.saturating_sub(1),
            _ => (self.menu_index + 1).min(count - 1),
        };
        vec![Effect::ScrollMenu(
            self.menu_index as f32 / (count.max(2) - 1) as f32,
        )]
    }

    /// Space or Enter: same effect as a click on the focused widget.
    pub fn activate(&mut self) -> Effects {
        let message = match self.focus.clone() {
            Some(Focus::Settings) => Message::Settings(SettingsMessage::Open),
            Some(Focus::CloseSettings) => Message::Settings(SettingsMessage::Close),
            Some(Focus::Language(choice)) => Message::Settings(SettingsMessage::Language(choice)),
            Some(Focus::Theme(choice)) => Message::Settings(SettingsMessage::Theme(choice)),
            Some(Focus::Internal) => Message::ToggleInternal(!self.show_internal),
            Some(Focus::Virtual) => Message::ToggleVirtual(!self.allow_virtual),
            Some(Focus::All) => Message::ToggleAll(!self.show_all),
            Some(Focus::Disk(id)) => Message::Select(id),
            Some(Focus::Details) => Message::ToggleDetails,
            Some(Focus::Copy) => Message::Copy,
            Some(Focus::Advanced) => Message::ToggleAdvanced,
            Some(Focus::Menu) => match (&self.catalog, self.menu_open) {
                (CatalogState::Ready(systems), true) => {
                    match writable(&self.menu_filter.apply(systems)).get(self.menu_index) {
                        Some(choice) => Message::MenuChose((*choice).clone()),
                        None => return Vec::new(),
                    }
                }
                (CatalogState::Ready(_), false) => Message::ToggleMenu,
                _ => return Vec::new(),
            },
            Some(Focus::Browse) => Message::BrowseImage,
            Some(Focus::Tor) if self.tor.is_some() => Message::ToggleTor(!self.use_tor),
            Some(Focus::Tor) => return Vec::new(),
            Some(Focus::Raw) => match &self.write {
                WriteState::Draft { raw, .. } => Message::ToggleRaw(!raw),
                _ => return Vec::new(),
            },
            Some(Focus::Mbr) => match &self.write {
                WriteState::Draft { mbr, .. } => Message::ToggleMbr(!mbr),
                _ => return Vec::new(),
            },
            // The image field takes the keys itself: Space and Enter activate nothing there.
            Some(Focus::Image | Focus::Hash) | None => return Vec::new(),
            Some(Focus::Action) => match self.write {
                WriteState::Draft { .. } => Message::StartWrite,
                WriteState::Running { .. } => Message::CancelWrite,
                WriteState::Finished { .. } => Message::DismissWrite,
                WriteState::Idle => return Vec::new(),
            },
        };
        self.apply(message)
    }
}
