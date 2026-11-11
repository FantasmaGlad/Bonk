mod app;
pub use bonk_ui::french_title;
mod locale;
mod theme;
mod transfer;

pub use bonk_ui::command::{ViewOptions, WriteView, equivalent_commands};
pub use bonk_ui::settings::ThemeChoice;

use iced::Size;

use crate::app::{App, Start};

/// With the starting display settings (`bonk config`); returns when the window closes.
pub fn run(
    initial: ViewOptions,
    lang: Option<bonk_hal::Lang>,
    theme: ThemeChoice,
    image: Option<String>,
) -> Result<(), String> {
    // Iced panics without a display instead of returning an error: detect it first so Bonk falls back cleanly
    // to the command line (SSH, server).
    if !display_available(|name| std::env::var_os(name)) {
        return Err("no graphical display (neither WAYLAND_DISPLAY nor DISPLAY)".to_owned());
    }
    run_with_fallback(
        move || {
            launch(Start {
                view: initial,
                lang,
                theme,
                image,
            })
        },
        fallback_allowed(|name| std::env::var_os(name)),
        relaunch_with_software_renderer,
    )
}

/// If `open` panics and the fallback is allowed, relaunches with software rendering (`relaunch`). Both steps
/// are injected to test this path without GPU or display: the Xvfb integration test only goes through it
/// where wgpu panics. An ordinary window error (no panic) is never retried.
fn run_with_fallback(
    open: impl FnOnce() -> Result<(), String> + std::panic::UnwindSafe,
    can_fall_back: bool,
    relaunch: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    match catching_panics(open) {
        Ok(result) => result,
        Err(message) if can_fall_back => {
            log::warn!(
                target: "bonk::gui", reason = message.as_str();
                "window panicked, restarting with the software renderer"
            );
            relaunch()
        }
        Err(message) => Err(message),
    }
}

/// Set on the process relaunched with software rendering: it does not relaunch itself.
const FALLBACK_MARKER: &str = "BONK_RENDER_FALLBACK";

/// Only when the user did not pick the renderer (`ICED_BACKEND`) and this is not already the fallback
/// process.
fn fallback_allowed(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    var("ICED_BACKEND").is_none() && var(FALLBACK_MARKER).is_none()
}

/// wgpu treats some driver errors as fatal (panic on the main thread) instead of letting Iced switch to
/// software rendering: under a virtual display, some VMs or an incomplete driver, the surface cannot be
/// configured.
fn catching_panics(
    open: impl FnOnce() -> Result<(), String> + std::panic::UnwindSafe,
) -> Result<Result<(), String>, String> {
    let previous = std::panic::take_hook();
    // Logged, not printed: it would duplicate the fallback, or be repeated in the final error if no fallback
    // is possible.
    std::panic::set_hook(Box::new(|info| {
        log::debug!(target: "bonk::gui", "panic in the window: {info}");
    }));
    let outcome = std::panic::catch_unwind(open);
    std::panic::set_hook(previous);
    outcome.map_err(|payload| panic_message(payload.as_ref()))
}

/// Raw RGBA: Iced does not decode PNG without one more dependency.
fn window_icon() -> Option<iced::window::Icon> {
    const SIDE: u32 = 64;
    let pixels = include_bytes!("../assets/bonk-64.rgba").to_vec();
    iced::window::icon::from_rgba(pixels, SIDE, SIDE).ok()
}

fn launch(start: Start) -> Result<(), String> {
    iced::application(move || App::new(start.clone()), App::update, App::view)
        .title("Bonk")
        // Shipped in the binary (SIL OFL 1.1, assets/fonts/OFL.txt): nothing is read from the system or the
        // network.
        .font(include_bytes!("../assets/fonts/FiraMono-Regular.ttf"))
        .default_font(iced::Font::with_name("Fira Sans"))
        .theme(App::theme)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            icon: window_icon(),
            ..iced::window::Settings::default()
        })
        .window_size(Size::new(960.0, 640.0))
        .centered()
        .run()
        .map_err(|erreur| erreur.to_string())
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panic without a message".to_owned())
}

/// Same arguments, software renderer (`tiny-skia`), waits for it. A new process rather than a variable set
/// here: changing the environment of a process that already has threads is unsafe, and the renderer state
/// that just panicked is not reusable.
fn relaunch_with_software_renderer() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("fallback impossible: {e}"))?;
    let status = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("ICED_BACKEND", "tiny-skia")
        .env(FALLBACK_MARKER, "1")
        .status()
        .map_err(|e| format!("fallback impossible: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the software renderer failed too ({status})"))
    }
}

/// On Unix other than macOS a graphical session is known by its environment variables; elsewhere the system
/// always provides a display.
#[cfg(all(unix, not(target_os = "macos")))]
fn display_available(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "DISPLAY"]
        .iter()
        .any(|name| var(name).is_some_and(|value| !value.is_empty()))
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn display_available(_var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    true
}

#[cfg(all(test, unix, not(target_os = "macos")))]
mod tests {
    use std::ffi::OsString;

    use super::display_available;

    fn avec(definies: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
        let definies: Vec<_> = definies.to_vec();
        move |nom| {
            definies
                .iter()
                .find(|(n, _)| *n == nom)
                .map(|(_, v)| OsString::from(v))
        }
    }

    #[test]
    fn le_repli_logiciel_n_est_permis_qu_une_fois_et_sans_choix_de_l_utilisateur() {
        let env = |definies: &'static [&'static str]| {
            move |nom: &str| definies.contains(&nom).then(|| OsString::from("x"))
        };
        assert!(super::fallback_allowed(env(&[])));
        // The user chose the renderer: do not override it.
        assert!(!super::fallback_allowed(env(&["ICED_BACKEND"])));
        // Already the fallback process: no relaunch loop.
        assert!(!super::fallback_allowed(env(&[super::FALLBACK_MARKER])));
    }

    #[test]
    fn message_d_une_panique() {
        let texte: Box<dyn std::any::Any + Send> = Box::new("wgpu error");
        let chaine: Box<dyn std::any::Any + Send> = Box::new(String::from("surface invalide"));
        let autre: Box<dyn std::any::Any + Send> = Box::new(42_u8);
        assert_eq!(super::panic_message(texte.as_ref()), "wgpu error");
        assert_eq!(super::panic_message(chaine.as_ref()), "surface invalide");
        assert_eq!(
            super::panic_message(autre.as_ref()),
            "panic without a message"
        );
    }

    #[test]
    fn une_panique_du_rendu_devient_une_erreur() {
        let hook_avant = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let capture = std::panic::catch_unwind(|| panic!("wgpu error: Invalid surface"));
        std::panic::set_hook(hook_avant);
        let message = capture.map_err(|p| super::panic_message(p.as_ref()));
        assert_eq!(message, Err("wgpu error: Invalid surface".to_owned()));
    }

    #[test]
    fn affichage_reconnu_par_ses_variables() {
        assert!(display_available(avec(&[("DISPLAY", ":0")])));
        assert!(display_available(avec(&[("WAYLAND_DISPLAY", "wayland-0")])));
        assert!(display_available(avec(&[("WAYLAND_SOCKET", "3")])));
    }

    #[test]
    fn sans_variable_ou_variable_vide_pas_d_affichage() {
        assert!(!display_available(avec(&[])));
        assert!(!display_available(avec(&[("DISPLAY", "")])));
    }
}

#[cfg(test)]
mod repli_tests {
    use std::cell::Cell;

    use super::run_with_fallback;

    #[test]
    fn une_fenetre_qui_s_ouvre_ne_relance_rien() {
        let relances = Cell::new(0);
        let resultat = run_with_fallback(
            || Ok(()),
            true,
            || {
                relances.set(relances.get() + 1);
                Ok(())
            },
        );
        assert_eq!(resultat, Ok(()));
        assert_eq!(relances.get(), 0);
    }

    #[test]
    fn une_erreur_ordinaire_n_est_pas_rejouee_en_logiciel() {
        let relances = Cell::new(0);
        let resultat = run_with_fallback(
            || Err("pas de rendu".to_owned()),
            true,
            || {
                relances.set(relances.get() + 1);
                Ok(())
            },
        );
        assert_eq!(resultat, Err("pas de rendu".to_owned()));
        assert_eq!(relances.get(), 0);
    }

    #[test]
    fn une_panique_relance_une_fois_en_logiciel() {
        let relances = Cell::new(0);
        let resultat = run_with_fallback(
            || panic!("wgpu error: Invalid surface"),
            true,
            || {
                relances.set(relances.get() + 1);
                Ok(())
            },
        );
        assert_eq!(resultat, Ok(()));
        assert_eq!(relances.get(), 1);
    }

    #[test]
    fn l_echec_du_rendu_logiciel_est_rendu_tel_quel() {
        let resultat = run_with_fallback(
            || panic!("wgpu error"),
            true,
            || Err("le rendu logiciel a échoué aussi".to_owned()),
        );
        assert_eq!(resultat, Err("le rendu logiciel a échoué aussi".to_owned()));
    }

    #[test]
    fn sans_repli_permis_la_panique_devient_l_erreur() {
        let relances = Cell::new(0);
        let resultat = run_with_fallback(
            || panic!("wgpu error: Invalid surface"),
            false,
            || {
                relances.set(relances.get() + 1);
                Ok(())
            },
        );
        assert_eq!(resultat, Err("wgpu error: Invalid surface".to_owned()));
        assert_eq!(relances.get(), 0);
    }
}

#[cfg(test)]
mod icon_tests {
    /// An icon of the wrong size would be lost silently: `window_icon` returns `None` then.
    #[test]
    fn l_icone_de_la_fenetre_est_valide() {
        assert!(super::window_icon().is_some());
    }
}
