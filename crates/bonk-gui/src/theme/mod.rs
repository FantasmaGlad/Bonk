mod palette;
mod widgets;

use iced::Theme;
use iced::theme::Palette;

pub(crate) use palette::{choose, colors};
pub(crate) use widgets::{
    card, card_button, checkbox_style, focus_ring, input_style, link_button, panel, primary_button,
};

/// Commands, paths, identifiers.
pub(crate) const MONO: iced::Font = iced::Font::with_name("Fira Mono");

/// Secondary text: hints, details, captions.
pub(crate) fn muted() -> iced::Color {
    colors().muted
}

pub(crate) fn bonk() -> Theme {
    let c = colors();
    Theme::custom(
        "Bonk".to_owned(),
        Palette {
            background: c.paper,
            text: c.ink,
            primary: c.accent,
            success: c.accent,
            warning: c.warn,
            danger: c.warn,
        },
    )
}
