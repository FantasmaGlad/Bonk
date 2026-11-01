//! Navigation au clavier : chaque touche a le même effet qu'un clic.

use iced::Task;
use iced::keyboard::{self, Key, key::Named};
use iced::widget::scrollable;

use super::catalog::{CatalogState, MENU, writable};
use super::{App, Focus, Message, Nav, WriteState};

/// Identifiant de la liste défilante, pour la suivre quand le focus descend dans la liste.
pub(super) const LIST: &str = "disques";

/// Touche pressée vers action : Tab et Maj+Tab, Espace et Entrée, flèches haut et bas, Échap.
/// Les combinaisons avec Ctrl, Alt ou Logo sont laissées au système (raccourcis).
pub(crate) fn map_key(key: &Key, modifiers: keyboard::Modifiers, repeat: bool) -> Option<Message> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    let nav = match key {
        Key::Named(Named::Tab) if modifiers.shift() => Nav::Previous,
        Key::Named(Named::Tab) => Nav::Next,
        // Une touche maintenue enfoncée ne doit pas cocher et décocher en boucle.
        Key::Named(Named::Space | Named::Enter) if !repeat => Nav::Activate,
        Key::Named(Named::ArrowUp) => Nav::Up,
        Key::Named(Named::ArrowDown) => Nav::Down,
        Key::Named(Named::Escape) => Nav::Escape,
        _ => return None,
    };
    Some(Message::Nav(nav))
}

/// Vrai si l'élément `i` de l'ordre de Tab est un disque (faux hors bornes).
pub(super) fn is_disk(order: &[Focus], i: usize) -> bool {
    matches!(order.get(i), Some(Focus::Disk(_)))
}

impl App {
    /// Composants focalisables dans l'ordre de Tab : les commandes, puis les disques affichés.
    pub(super) fn focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::Refresh, Focus::Internal, Focus::Virtual, Focus::All];
        if self.error.is_none() {
            for (disk, _) in self.entries() {
                order.push(Focus::Disk(disk.id.clone()));
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
        // Hors de la liste : le disque d'une écriture en cours ou finie a pu partir.
        if matches!(
            self.write,
            WriteState::Running { .. } | WriteState::Finished { .. }
        ) {
            order.push(Focus::Action);
        }
        order.push(Focus::Copy);
        order
    }

    /// Le focus ne reste pas sur un disque qui a disparu de la liste (débranché ou masqué).
    pub(super) fn fix_focus(&mut self) {
        if self
            .focus
            .as_ref()
            .is_some_and(|focus| !self.focus_order().contains(focus))
        {
            self.focus = None;
        }
    }

    pub(super) fn navigate(&mut self, nav: Nav) -> Task<Message> {
        // Devant la demande de confirmation, le clavier ne peut que renoncer : Entrée ne doit pas effacer.
        if self.confirming_internal {
            return if nav == Nav::Escape {
                self.apply(Message::CloseInternal)
            } else {
                Task::none()
            };
        }
        // Menu ouvert : les flèches y circulent, Échap le referme.
        if self.menu_open {
            match nav {
                Nav::Up | Nav::Down => return self.move_in_menu(nav),
                Nav::Escape => {
                    self.menu_open = false;
                    return Task::none();
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
            // Les flèches se déplacent entre les disques ; depuis une commande, Bas entre dans la liste.
            Nav::Down => match current {
                Some(i)
                    if !matches!(
                        order[i],
                        Focus::Refresh
                            | Focus::Internal
                            | Focus::Virtual
                            | Focus::All
                            | Focus::Copy
                    ) =>
                {
                    // Disque suivant, en sautant le panneau d'écriture du disque courant.
                    (i + 1..order.len())
                        .find(|j| is_disk(&order, *j))
                        .or(Some(i))
                }
                _ => order.iter().position(|item| matches!(item, Focus::Disk(_))),
            },
            // Haut remonte d'un disque ; depuis le bouton de copie, il revient au dernier disque.
            Nav::Up => match current {
                Some(i)
                    if matches!(
                        order[i],
                        Focus::Disk(_)
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
                    // Disque précédent ; depuis les panneaux et le bouton de copie, le disque le plus proche.
                    let from_disk = matches!(order[i], Focus::Disk(_));
                    (0..i)
                        .rev()
                        .find(|j| is_disk(&order, *j))
                        .filter(|j| *j >= 4 || !from_disk)
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
            return Task::none();
        };
        self.focus = Some(order[index].clone());
        // La liste suit le focus : position relative de la carte parmi les disques (approximative si
        // des détails sont ouverts, suffisante pour garder la carte visible).
        let disk_positions: Vec<usize> = (0..order.len()).filter(|i| is_disk(&order, *i)).collect();
        let rank = disk_positions.iter().position(|i| *i == index);
        if let Some(rank) = rank.filter(|_| disk_positions.len() > 1) {
            let relative = rank as f32 / (disk_positions.len() - 1) as f32;
            return iced::widget::operation::snap_to(
                LIST,
                scrollable::RelativeOffset {
                    x: 0.0,
                    y: relative,
                },
            );
        }
        Task::none()
    }

    /// Déplace la ligne désignée et garde la liste à sa hauteur.
    fn move_in_menu(&mut self, nav: Nav) -> Task<Message> {
        let CatalogState::Ready(systems) = &self.catalog else {
            return Task::none();
        };
        let count = writable(&self.menu_filter.apply(systems)).len();
        if count == 0 {
            return Task::none();
        }
        self.menu_index = match nav {
            Nav::Up => self.menu_index.saturating_sub(1),
            _ => (self.menu_index + 1).min(count - 1),
        };
        iced::widget::operation::snap_to(
            MENU,
            scrollable::RelativeOffset {
                x: 0.0,
                y: self.menu_index as f32 / (count.max(2) - 1) as f32,
            },
        )
    }

    /// Espace ou Entrée : même effet qu'un clic sur le composant qui a le focus.
    pub(super) fn activate(&mut self) -> Task<Message> {
        let message = match self.focus.clone() {
            Some(Focus::Refresh) => Message::Refresh,
            Some(Focus::Internal) => Message::ToggleInternal(!self.show_internal),
            Some(Focus::Virtual) => Message::ToggleVirtual(!self.allow_virtual),
            Some(Focus::All) => Message::ToggleAll(!self.show_all),
            Some(Focus::Disk(id)) => Message::Select(id),
            Some(Focus::Copy) => Message::Copy,
            Some(Focus::Advanced) => Message::ToggleAdvanced,
            Some(Focus::Menu) => match (&self.catalog, self.menu_open) {
                (CatalogState::Ready(systems), true) => {
                    match writable(&self.menu_filter.apply(systems)).get(self.menu_index) {
                        Some(choice) => Message::MenuChose((*choice).clone()),
                        None => return Task::none(),
                    }
                }
                (CatalogState::Ready(_), false) => Message::ToggleMenu,
                _ => return Task::none(),
            },
            Some(Focus::Browse) => Message::BrowseImage,
            Some(Focus::Tor) if self.tor.is_some() => Message::ToggleTor(!self.use_tor),
            Some(Focus::Tor) => return Task::none(),
            Some(Focus::Raw) => match &self.write {
                WriteState::Draft { raw, .. } => Message::ToggleRaw(!raw),
                _ => return Task::none(),
            },
            Some(Focus::Mbr) => match &self.write {
                WriteState::Draft { mbr, .. } => Message::ToggleMbr(!mbr),
                _ => return Task::none(),
            },
            // Le champ de l'image reçoit les touches lui-même : Espace et Entrée n'y activent rien.
            Some(Focus::Image | Focus::Hash) | None => return Task::none(),
            Some(Focus::Action) => match self.write {
                WriteState::Draft { .. } => Message::StartWrite,
                WriteState::Running { .. } => Message::CancelWrite,
                WriteState::Finished { .. } => Message::DismissWrite,
                WriteState::Idle => return Task::none(),
            },
        };
        self.apply(message)
    }
}
