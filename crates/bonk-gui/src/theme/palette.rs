use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, OnceLock};

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

/// A theme and its incognito variant, switched without a relaunch when Tor is turned on.
#[derive(Debug, PartialEq)]
struct Family {
    plain: Colors,
    incognito: Colors,
}

impl Family {
    fn read(dark: bool) -> Self {
        let (plain, incognito) = if dark {
            ("dark", "incognito_dark")
        } else {
            ("light", "incognito_light")
        };
        Self {
            plain: read(plain),
            incognito: read(incognito),
        }
    }

    fn colors(&self, incognito: bool) -> &Colors {
        if incognito {
            &self.incognito
        } else {
            &self.plain
        }
    }
}

/// One family per run: the other is never built, and the window relaunches to change it.
struct Chosen(OnceLock<Family>);

impl Chosen {
    const fn new() -> Self {
        Self(OnceLock::new())
    }

    fn choose(&self, dark: bool) {
        self.0.get_or_init(|| Family::read(dark));
    }

    fn family(&self) -> &Family {
        self.0.get_or_init(|| Family::read(false))
    }
}

static CHOSEN: Chosen = Chosen::new();
static INCOGNITO: AtomicBool = AtomicBool::new(false);

/// Before the window opens; later calls change nothing.
pub(crate) fn choose(dark: bool) {
    CHOSEN.choose(dark);
}

pub(crate) fn set_incognito(on: bool) {
    INCOGNITO.store(on, Ordering::Relaxed);
}

pub(crate) fn colors() -> &'static Colors {
    #[cfg(test)]
    if let Some(colors) = tests::drawn() {
        return colors;
    }
    CHOSEN.family().colors(INCOGNITO.load(Ordering::Relaxed))
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
pub(crate) mod tests {
    use std::cell::Cell;

    use super::*;

    thread_local! {
        static DRAWN: Cell<Option<&'static Colors>> = const { Cell::new(None) };
    }

    pub(super) fn drawn() -> Option<&'static Colors> {
        DRAWN.with(Cell::get)
    }

    /// Captures draw on the test's own thread: each picks its palette without touching the run's.
    pub(crate) fn draw_with<T>(palette: &str, capture: impl FnOnce() -> T) -> T {
        let colors: &'static Colors = Box::leak(Box::new(read(palette)));
        DRAWN.with(|drawn| drawn.set(Some(colors)));
        let result = capture();
        DRAWN.with(|drawn| drawn.set(None));
        result
    }

    #[test]
    fn la_premiere_famille_choisie_reste_pour_toute_l_execution() {
        let chosen = Chosen::new();
        chosen.choose(true);
        chosen.choose(false);
        assert_eq!(*chosen.family(), Family::read(true));
    }

    #[test]
    fn sans_choix_la_fenetre_est_claire_et_le_reste() {
        let chosen = Chosen::new();
        assert_eq!(*chosen.family(), Family::read(false));
        chosen.choose(true);
        assert_eq!(*chosen.family(), Family::read(false));
    }

    #[test]
    fn chaque_famille_porte_sa_variante_incognito() {
        let dark = Family::read(true);
        assert_eq!(*dark.colors(false), read("dark"));
        assert_eq!(*dark.colors(true), read("incognito_dark"));
        let light = Family::read(false);
        assert_eq!(*light.colors(true), read("incognito_light"));
    }

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

    /// Text 4.5:1 on every background it sits on, in every palette; a missing or malformed colour fails here.
    #[test]
    fn chaque_palette_tient_les_contrastes() {
        for theme in ["light", "dark", "incognito_light", "incognito_dark"] {
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
