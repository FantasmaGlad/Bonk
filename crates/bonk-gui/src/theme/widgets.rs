use iced::widget::{button, checkbox, container, text_input};
use iced::{Border, Color, Theme};

use super::palette::{RADIUS, colors};

fn border(color: Color, width: f32) -> Border {
    Border {
        color,
        width,
        radius: (*RADIUS).into(),
    }
}

fn pressed(status: button::Status) -> bool {
    matches!(status, button::Status::Hovered | button::Status::Pressed)
}

/// The one action that matters on a view: accent background.
pub(crate) fn primary_button(_theme: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    let background = match status {
        button::Status::Disabled => c.muted,
        status if pressed(status) => c.accent_strong,
        _ => c.accent,
    };
    button::Style {
        background: Some(background.into()),
        text_color: c.on_accent,
        border: border(background, 0.0),
        ..button::Style::default()
    }
}

/// A clickable card or choice: its own outline marks selection and hover, never a second frame around it.
/// The outline keeps 3:1 against the page (WCAG 1.4.11).
pub(crate) fn card_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    card(selected, false)
}

/// As [`card_button`], with keyboard focus drawn by the outline itself: a card has no ring around it.
pub(crate) fn card(
    selected: bool,
    focused: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let c = colors();
        let (outline, width) = if focused {
            (c.accent, 3.0)
        } else if selected {
            (c.accent, 2.0)
        } else if pressed(status) {
            (c.accent, 1.0)
        } else {
            (c.muted, 1.0)
        };
        button::Style {
            background: Some(if selected { c.accent_soft } else { c.surface }.into()),
            text_color: c.ink,
            border: border(outline, width),
            ..button::Style::default()
        }
    }
}

/// A raised area without outline: the surface colour alone sets it apart from the page.
pub(crate) fn panel(_theme: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(c.surface.into()),
        border: border(Color::TRANSPARENT, 0.0),
        ..container::Style::default()
    }
}

pub(crate) fn checkbox_style(_theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let c = colors();
    let checked = match status {
        checkbox::Status::Active { is_checked }
        | checkbox::Status::Hovered { is_checked }
        | checkbox::Status::Disabled { is_checked } => is_checked,
    };
    checkbox::Style {
        background: if checked { c.accent } else { c.surface }.into(),
        icon_color: c.on_accent,
        // Square enough to read as a box, not as a round choice.
        border: Border {
            color: if checked { c.accent } else { c.muted },
            width: 2.0,
            radius: 4.0.into(),
        },
        text_color: Some(c.ink),
    }
}

/// Transparent without focus: the layout does not move when focus moves.
pub(crate) fn focus_ring(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        border: Border {
            color: if focused {
                colors().accent
            } else {
                Color::TRANSPARENT
            },
            width: 2.0,
            radius: (*RADIUS + 2.0).into(),
        },
        ..container::Style::default()
    }
}

pub(crate) fn input_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    let c = colors();
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: c.surface.into(),
        border: border(
            if focused { c.accent } else { c.muted },
            if focused { 2.0 } else { 1.0 },
        ),
        icon: c.muted,
        placeholder: c.muted,
        value: c.ink,
        selection: c.accent_soft,
    }
}

/// A quiet action in running text: accent text, no outline.
pub(crate) fn link_button(_theme: &Theme, status: button::Status) -> button::Style {
    let c = colors();
    button::Style {
        background: None,
        text_color: if pressed(status) {
            c.accent_strong
        } else {
            c.accent
        },
        border: border(Color::TRANSPARENT, 0.0),
        ..button::Style::default()
    }
}

/// Before erasing: the alert colours of the site, readable in both themes.
pub(crate) fn warning_banner(_theme: &Theme) -> container::Style {
    let c = colors();
    container::Style {
        background: Some(c.warn_soft.into()),
        text_color: Some(c.warn),
        border: border(Color::TRANSPARENT, 0.0),
        ..container::Style::default()
    }
}
