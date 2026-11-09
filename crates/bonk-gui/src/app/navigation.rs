use iced::keyboard::{self, Key, key::Named};

use super::{Message, Nav};

/// Id of the scrolling list, to follow it when focus moves into the disks.
pub(super) const LIST: &str = "disques";

/// Tab and Shift+Tab, Space and Enter, arrows, Escape. Combinations with Ctrl, Alt or Logo are left to the
/// system (shortcuts).
pub(crate) fn map_key(key: &Key, modifiers: keyboard::Modifiers, repeat: bool) -> Option<Message> {
    if modifiers.control() || modifiers.alt() || modifiers.logo() {
        return None;
    }
    let nav = match key {
        Key::Named(Named::Tab) if modifiers.shift() => Nav::Previous,
        Key::Named(Named::Tab) => Nav::Next,
        // A held key must not check and uncheck in a loop.
        Key::Named(Named::Space | Named::Enter) if !repeat => Nav::Activate,
        Key::Named(Named::ArrowUp) => Nav::Up,
        Key::Named(Named::ArrowDown) => Nav::Down,
        Key::Named(Named::Escape) => Nav::Escape,
        _ => return None,
    };
    Some(Message::Nav(nav))
}
