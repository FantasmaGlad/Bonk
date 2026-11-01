//! Touches du clavier vers actions de la fenêtre.

use iced::keyboard::{self, Key, key::Named};

use super::{Message, Nav};

/// Identifiant de la liste défilante, pour la suivre quand le focus descend dans les disques.
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
