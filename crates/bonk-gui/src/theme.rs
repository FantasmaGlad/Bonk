use iced::theme::Palette;
use iced::widget::{button, checkbox, container, text_input};
use iced::{Border, Color, Theme};

pub(crate) const BACKGROUND: Color = Color::WHITE;
/// `#000000` (21:1 on the background).
pub(crate) const TEXT: Color = Color::BLACK;
/// `#333333` (12.6:1 on the background).
pub(crate) const SECONDARY: Color = Color::from_rgb(0.2, 0.2, 0.2);
/// `#000000` (21:1; WCAG 1.4.11 asks for 3:1).
pub(crate) const CONTROL_BORDER: Color = Color::BLACK;
/// `#E5E5E5` (black text on it: 16.7:1).
pub(crate) const HOVER: Color = Color::from_rgb(0.898, 0.898, 0.898);

const RADIUS: f32 = 2.0;

pub(crate) fn bonk() -> Theme {
    Theme::custom(
        "Bonk".to_owned(),
        Palette {
            background: BACKGROUND,
            text: TEXT,
            primary: TEXT,
            success: TEXT,
            warning: TEXT,
            danger: TEXT,
        },
    )
}

/// Black background, white text (21:1); dark grey on hover.
pub(crate) fn primary_button(_theme: &Theme, status: button::Status) -> button::Style {
    let background = match status {
        button::Status::Hovered | button::Status::Pressed => SECONDARY,
        _ => TEXT,
    };
    button::Style {
        background: Some(background.into()),
        text_color: BACKGROUND,
        border: Border {
            color: TEXT,
            width: 2.0,
            radius: RADIUS.into(),
        },
        ..button::Style::default()
    }
}

/// 1 px black border; selection and hover thicken it to 3 px (hover also greys the background), never relying
/// on colour.
pub(crate) fn card_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: Some(if hovered { HOVER } else { BACKGROUND }.into()),
            text_color: TEXT,
            border: Border {
                color: CONTROL_BORDER,
                width: if selected || hovered { 3.0 } else { 1.0 },
                radius: RADIUS.into(),
            },
            ..button::Style::default()
        }
    }
}

/// 2 px black border, to stand out without colour.
pub(crate) fn panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(BACKGROUND.into()),
        border: Border {
            color: CONTROL_BORDER,
            width: 2.0,
            radius: RADIUS.into(),
        },
        ..container::Style::default()
    }
}

/// Black border even unchecked (a checkbox is a component, WCAG 1.4.11), black fill and white tick when
/// checked.
pub(crate) fn checkbox_style(_theme: &Theme, status: checkbox::Status) -> checkbox::Style {
    let checked = match status {
        checkbox::Status::Active { is_checked }
        | checkbox::Status::Hovered { is_checked }
        | checkbox::Status::Disabled { is_checked } => is_checked,
    };
    checkbox::Style {
        background: if checked { TEXT } else { BACKGROUND }.into(),
        icon_color: BACKGROUND,
        border: Border {
            color: CONTROL_BORDER,
            width: 2.0,
            radius: RADIUS.into(),
        },
        text_color: Some(TEXT),
    }
}

/// 3 px ring in the text colour, separated from the widget by a gap (the widget may already be black).
/// Transparent without focus: the layout does not move when focus moves.
pub(crate) fn focus_ring(focused: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        border: Border {
            color: if focused { TEXT } else { Color::TRANSPARENT },
            width: 3.0,
            radius: (RADIUS + 2.0).into(),
        },
        ..container::Style::default()
    }
}

/// White background, 2 px black border (an input is a component, WCAG 1.4.11).
pub(crate) fn input_style(_theme: &Theme, status: text_input::Status) -> text_input::Style {
    text_input::Style {
        background: BACKGROUND.into(),
        border: Border {
            color: CONTROL_BORDER,
            width: if matches!(status, text_input::Status::Focused { .. }) {
                3.0
            } else {
                2.0
            },
            radius: RADIUS.into(),
        },
        icon: TEXT,
        placeholder: SECONDARY,
        value: TEXT,
        selection: HOVER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_charte_suit_le_fichier_commun() {
        let tokens: serde_json::Value =
            serde_json::from_str(include_str!("../../../design/tokens.json")).unwrap_or_default();
        let hex = |color: Color| {
            let [r, g, b, _] = color.into_rgba8();
            format!("#{r:02X}{g:02X}{b:02X}")
        };
        for (name, color) in [
            ("paper", BACKGROUND),
            ("ink", TEXT),
            ("secondary", SECONDARY),
            ("hover", HOVER),
        ] {
            assert_eq!(tokens["color"][name], hex(color), "{name}");
        }
        assert_eq!(tokens["radius"].as_f64(), Some(f64::from(RADIUS)));
    }
}
