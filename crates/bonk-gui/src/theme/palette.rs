use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use iced::Color;

/// The colours of one theme, read from `design/tokens.json`, the file Android reads too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Colors {
    pub(crate) paper: Color,
    pub(crate) surface: Color,
    pub(crate) sunk: Color,
    pub(crate) ink: Color,
    pub(crate) muted: Color,
    pub(crate) accent: Color,
    pub(crate) accent_strong: Color,
    pub(crate) accent_soft: Color,
    pub(crate) warn: Color,
    pub(crate) warn_soft: Color,
    pub(crate) line: Color,
    pub(crate) on_accent: Color,
}

const TOKENS: &str = include_str!("../../../../design/tokens.json");

/// Corner radius shared with the site.
pub(crate) static RADIUS: LazyLock<f32> = LazyLock::new(|| {
    let tokens: serde_json::Value = serde_json::from_str(TOKENS).unwrap_or_default();
    // An unreadable file draws square corners: the test below fails on it first.
    tokens["radius"].as_f64().unwrap_or_default() as f32
});

static LIGHT: LazyLock<Colors> = LazyLock::new(|| read("light"));
static DARK: LazyLock<Colors> = LazyLock::new(|| read("dark"));

/// Chosen once, before the first frame that knows it; the window relaunches to change it.
static DARK_CHOSEN: AtomicBool = AtomicBool::new(false);

pub(crate) fn choose(dark: bool) {
    DARK_CHOSEN.store(dark, Ordering::Relaxed);
}

pub(crate) fn colors() -> &'static Colors {
    if DARK_CHOSEN.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

fn read(theme: &str) -> Colors {
    let tokens: serde_json::Value = serde_json::from_str(TOKENS).unwrap_or_default();
    let color = |name: &str| {
        tokens[theme][name]
            .as_str()
            .and_then(parse_hex)
            .unwrap_or(Color::BLACK)
    };
    Colors {
        paper: color("paper"),
        surface: color("surface"),
        sunk: color("sunk"),
        ink: color("ink"),
        muted: color("muted"),
        accent: color("accent"),
        accent_strong: color("accent_strong"),
        accent_soft: color("accent_soft"),
        warn: color("warn"),
        warn_soft: color("warn_soft"),
        line: color("line"),
        on_accent: color("on_accent"),
    }
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digits = hex.strip_prefix('#')?;
    let value = u32::from_str_radix(digits, 16).ok()?;
    let [_, r, g, b] = value.to_be_bytes();
    (digits.len() == 6).then(|| Color::from_rgb8(r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG 2.1 relative luminance.
    fn luminance(color: Color) -> f32 {
        let channel = |c: f32| {
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(color.r) + 0.7152 * channel(color.g) + 0.0722 * channel(color.b)
    }

    fn contrast(a: Color, b: Color) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Text 4.5:1 on every background it sits on, in both themes; a missing or malformed colour fails here.
    #[test]
    fn les_deux_themes_tiennent_les_contrastes() {
        for theme in ["light", "dark"] {
            let c = read(theme);
            for (front, back) in [
                (c.ink, c.paper),
                (c.ink, c.surface),
                (c.ink, c.sunk),
                (c.ink, c.accent_soft),
                (c.muted, c.paper),
                (c.muted, c.surface),
                (c.muted, c.sunk),
                (c.accent, c.paper),
                (c.accent, c.surface),
                (c.accent, c.accent_soft),
                (c.on_accent, c.accent),
                (c.on_accent, c.accent_strong),
                (c.warn, c.warn_soft),
                (c.warn, c.paper),
            ] {
                assert!(
                    contrast(front, back) >= 4.5,
                    "{theme} : {front:?} sur {back:?} ({:.2})",
                    contrast(front, back)
                );
            }
        }
        assert!((*RADIUS - 8.0).abs() < f32::EPSILON);
    }

    #[test]
    fn couleur_hexadecimale() {
        assert_eq!(
            parse_hex("#FAF7F2"),
            Some(Color::from_rgb8(0xFA, 0xF7, 0xF2))
        );
        assert_eq!(parse_hex("FAF7F2"), None);
        assert_eq!(parse_hex("#FFF"), None);
    }
}
