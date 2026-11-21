mod icons;
mod palette;
mod widgets;

use iced::Theme;
use iced::theme::Palette;

pub(crate) use icons::incognito as incognito_icon;
#[cfg(test)]
pub(crate) use palette::tests as palette_tests;
pub(crate) use palette::{choose, colors, set_incognito};
pub(crate) use widgets::{
    card, card_button, checkbox_style, focus_ring, input_style, link_button, panel, primary_button,
    round_button, warning_banner,
};

pub(crate) const MONO: iced::Font = iced::Font::with_name("Fira Mono");

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
