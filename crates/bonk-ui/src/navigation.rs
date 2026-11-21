use crate::browse::BrowseMessage;
use crate::catalog::CatalogState;
use crate::library::LibraryMessage;
use crate::model::{Effect, Effects, Focus, Message, Model, Nav, WriteState};
use crate::settings::{Screen, SettingsMessage};

pub fn is_disk(order: &[Focus], i: usize) -> bool {
    matches!(order.get(i), Some(Focus::Disk(_)))
}

impl Model {
    pub fn focus_order(&self) -> Vec<Focus> {
        match self.screen {
            Screen::Settings => return self.settings_focus_order(),
            Screen::Library => return self.library_focus_order(),
            Screen::Catalog => return self.catalog_focus_order(),
            Screen::Main => {}
        }
        let mut order = Vec::new();
        if !self.offline {
            order.extend([Focus::Tor, Focus::Catalog]);
        }
        order.extend([Focus::Library, Focus::Settings]);
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
        if let Some(effects) = self.navigate_overlay(nav) {
            return effects;
        }
        let order = self.focus_order();
        let current = self
            .focus
            .as_ref()
            .and_then(|focus| order.iter().position(|item| item == focus));
        let target = match nav {
            Nav::Activate => return self.activate(),
            Nav::Escape => {
                if self.selected.is_some() {
                    self.selected = None;
                } else {
                    self.focus = None;
                }
                None
            }
            Nav::Next | Nav::Previous | Nav::Down | Nav::Up => target(&order, current, nav),
        };
        let Some(index) = target else {
            return Vec::new();
        };
        self.focus = Some(order[index].clone());
        follow(&order, index)
    }

    fn navigate_overlay(&mut self, nav: Nav) -> Option<Effects> {
        // In front of the confirmation, the keyboard can only give up: Enter must not erase.
        if self.confirming_internal {
            return Some(if nav == Nav::Escape {
                self.apply(Message::CloseInternal)
            } else {
                Vec::new()
            });
        }
        match (nav, self.screen) {
            (Nav::Escape, Screen::Settings) => {
                return Some(self.apply(Message::Settings(SettingsMessage::Close)));
            }
            (Nav::Escape, Screen::Library) => {
                return Some(self.apply(Message::Library(LibraryMessage::Close)));
            }
            (Nav::Escape, Screen::Catalog) => {
                let back = if self.browse.opened.is_some() {
                    BrowseMessage::CloseSystem
                } else {
                    BrowseMessage::Close
                };
                return Some(self.apply(Message::Browse(back)));
            }
            _ => {}
        }
        if !self.menu_open {
            return None;
        }
        match nav {
            Nav::Up | Nav::Down => Some(self.move_in_menu(nav)),
            Nav::Escape => {
                self.menu_open = false;
                Some(Vec::new())
            }
            _ => None,
        }
    }

    fn move_in_menu(&mut self, nav: Nav) -> Effects {
        let CatalogState::Ready(systems) = &self.catalog else {
            return Vec::new();
        };
        let count = self.menu_filter.writable(systems).len();
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

    pub fn activate(&mut self) -> Effects {
        let message = match self.focus.clone() {
            Some(Focus::Settings) => Message::Settings(SettingsMessage::Open),
            Some(Focus::CloseSettings) => Message::Settings(SettingsMessage::Close),
            Some(Focus::Library) => Message::Library(LibraryMessage::Open),
            Some(Focus::CloseLibrary) => Message::Library(LibraryMessage::Close),
            Some(Focus::Catalog) => Message::Browse(BrowseMessage::Open),
            Some(Focus::CloseCatalog) => Message::Browse(BrowseMessage::Close),
            Some(Focus::System(name)) => Message::Browse(BrowseMessage::OpenSystem(name)),
            Some(Focus::CloseSystem) => Message::Browse(BrowseMessage::CloseSystem),
            Some(Focus::Variant(id)) => Message::Browse(BrowseMessage::Write(id)),
            Some(Focus::Download(id)) => Message::Browse(BrowseMessage::Download(id)),
            Some(Focus::DownloadAction) => Message::Browse(
                if self
                    .browse
                    .download
                    .as_ref()
                    .is_some_and(crate::browse::DownloadState::running)
                {
                    BrowseMessage::CancelDownload
                } else {
                    BrowseMessage::DismissDownload
                },
            ),
            Some(Focus::CatalogSearch) => return Vec::new(),
            Some(Focus::KeptWrite(sha256)) => Message::Library(LibraryMessage::Write(sha256)),
            Some(Focus::KeptRemove(sha256)) => Message::Library(LibraryMessage::Remove(sha256)),
            Some(Focus::Language(choice)) => Message::Settings(SettingsMessage::Language(choice)),
            Some(Focus::Theme(choice)) => Message::Settings(SettingsMessage::Theme(choice)),
            Some(Focus::Internal) => Message::ToggleInternal(!self.show_internal),
            Some(Focus::Virtual) => Message::ToggleVirtual(!self.allow_virtual),
            Some(Focus::All) => Message::ToggleAll(!self.show_all),
            Some(Focus::Offline) => Message::Settings(SettingsMessage::Offline(!self.offline)),
            Some(Focus::IncognitoAtStart) => {
                Message::Settings(SettingsMessage::IncognitoAtStart(!self.incognito_at_start))
            }
            Some(Focus::Notify) => Message::Settings(SettingsMessage::Notify(!self.notify)),
            Some(Focus::Disk(id)) => Message::Select(id),
            Some(Focus::Details) => Message::ToggleDetails,
            Some(Focus::Copy) => Message::Copy,
            Some(Focus::Advanced) => Message::ToggleAdvanced,
            Some(Focus::Menu) => match (&self.catalog, self.menu_open) {
                (CatalogState::Ready(systems), true) => {
                    match self.menu_filter.writable(systems).get(self.menu_index) {
                        Some(choice) => Message::MenuChose(Box::new((*choice).clone())),
                        None => return Vec::new(),
                    }
                }
                (CatalogState::Ready(_), false) => Message::ToggleMenu,
                _ => return Vec::new(),
            },
            Some(Focus::Browse) => Message::BrowseImage,
            Some(Focus::Tor) if self.tor.is_some() => Message::ToggleTor(!self.use_tor),
            Some(Focus::Tor) => Message::OpenTorProject,
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

fn target(order: &[Focus], current: Option<usize>, nav: Nav) -> Option<usize> {
    let last = order.len() - 1;
    match nav {
        Nav::Next => Some(current.map_or(0, |i| if i == last { 0 } else { i + 1 })),
        Nav::Previous => Some(current.map_or(last, |i| if i == 0 { last } else { i - 1 })),
        Nav::Down => match current {
            Some(i)
                if !matches!(
                    order[i],
                    Focus::Tor | Focus::Catalog | Focus::Library | Focus::Settings | Focus::Copy
                ) =>
            {
                (i + 1..order.len())
                    .find(|j| is_disk(order, *j))
                    .or(Some(i))
            }
            _ => order.iter().position(|item| matches!(item, Focus::Disk(_))),
        },
        Nav::Up => match current {
            Some(i) if in_disk_area(&order[i]) => (0..i).rev().find(|j| is_disk(order, *j)),
            _ => None,
        },
        Nav::Activate | Nav::Escape => None,
    }
}

fn in_disk_area(focus: &Focus) -> bool {
    matches!(
        focus,
        Focus::Disk(_)
            | Focus::Details
            | Focus::Menu
            | Focus::Advanced
            | Focus::Image
            | Focus::Browse
            | Focus::Hash
            | Focus::Raw
            | Focus::Mbr
            | Focus::Action
            | Focus::Copy
    )
}

/// The list follows the focus: the card's position among the disks (approximate with open details, enough
/// to keep it visible).
fn follow(order: &[Focus], index: usize) -> Effects {
    let disk_positions: Vec<usize> = (0..order.len()).filter(|i| is_disk(order, *i)).collect();
    let rank = disk_positions.iter().position(|i| *i == index);
    match rank.filter(|_| disk_positions.len() > 1) {
        Some(rank) => vec![Effect::ScrollList(
            rank as f32 / (disk_positions.len() - 1) as f32,
        )],
        None => Vec::new(),
    }
}
