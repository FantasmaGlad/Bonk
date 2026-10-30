//! Charte graphique : thème clair par défaut, noir sur blanc, contraste maximal, angles vifs.
//!
//! Choix du mainteneur (2026-10-29) : thème clair, à la place du thème sombre du CDC §8 ; la règle
//! d'accessibilité du CDC est tenue avec de la marge (contours de composants à 21:1 pour 3:1 exigés).
//! Les états ne reposent jamais sur la couleur : ils sont écrits en toutes lettres.

use iced::theme::Palette;
use iced::widget::{button, checkbox, container, text_input};
use iced::{Border, Color, Theme};

/// Fond principal `#FFFFFF`.
pub(crate) const BACKGROUND: Color = Color::WHITE;
/// Texte et éléments actifs `#000000` (21:1 sur le fond).
pub(crate) const TEXT: Color = Color::BLACK;
/// Texte secondaire `#333333` (12,6:1 sur le fond).
pub(crate) const SECONDARY: Color = Color::from_rgb(0.2, 0.2, 0.2);
/// Contour de composant interactif `#000000` (21:1 ; WCAG 1.4.11 exige 3:1).
pub(crate) const CONTROL_BORDER: Color = Color::BLACK;
/// Fond d'un composant survolé `#E5E5E5` (texte noir dessus : 16,7:1).
pub(crate) const HOVER: Color = Color::from_rgb(0.898, 0.898, 0.898);

/// Rayon maximal de la charte : 4 px.
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

/// Bouton primaire : fond noir, texte blanc (21:1) ; gris foncé au survol.
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

/// Carte de disque cliquable : contour noir de 1 px ; la sélection et le survol l'épaississent à 3 px
/// (le survol grise en plus le fond), sans reposer sur la couleur.
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

/// Panneau d'information (erreur) : contour noir de 2 px, pour se distinguer sans couleur.
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

/// Case à cocher : contour noir même décochée (une case est un composant, WCAG 1.4.11), fond noir et
/// coche blanche une fois cochée.
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

/// Indicateur de focus clavier : contour de 3 px de la couleur du texte, séparé du composant par un
/// espace (le composant lui-même peut déjà être noir). Hors focus, le contour est transparent : la mise
/// en page ne bouge pas quand le focus se déplace.
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

/// Champ de saisie : fond blanc, contour noir de 2 px (un champ est un composant, WCAG 1.4.11).
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
