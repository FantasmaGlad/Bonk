//! Interface graphique de Bonk (Iced) : pour l'instant, une fenêtre de détection des disques en
//! lecture seule.
//!
//! Ce crate et `bonk-cli` ne se connaissent pas : tous deux s'appuient sur les mêmes crates de base.
//! Le rendu utilise wgpu et bascule sur le moteur logiciel tiny-skia quand aucun pilote graphique
//! n'est utilisable (CDC §8).

mod app;
mod command;
mod theme;
mod transfer;

pub use command::{ViewOptions, equivalent_commands};

use iced::Size;

use crate::app::App;

/// Ouvre la fenêtre, avec les réglages d'affichage de départ (`bonk config`), et rend la main à la
/// fermeture.
///
/// # Errors
/// Texte de l'erreur d'Iced quand la fenêtre ne peut pas s'ouvrir (pas d'affichage, pas de rendu).
pub fn run(initial: ViewOptions) -> Result<(), String> {
    // Iced ne renvoie pas d'erreur sans affichage : il panique. Détection de la fonctionnalité en
    // amont, pour que Bonk redirige proprement vers la ligne de commande (SSH, serveur).
    if !display_available(|name| std::env::var_os(name)) {
        return Err("aucun affichage graphique (ni WAYLAND_DISPLAY ni DISPLAY)".to_owned());
    }
    run_with_fallback(
        move || launch(initial),
        fallback_allowed(|name| std::env::var_os(name)),
        relaunch_with_software_renderer,
    )
}

/// Ouvre la fenêtre (`open`) ; si elle panique et que le repli est permis, relance en rendu logiciel
/// (`relaunch`). Les deux étapes sont injectées pour éprouver ce chemin sans GPU ni affichage : le test
/// d'intégration sous Xvfb ne le parcourt que là où wgpu panique. Une erreur ordinaire de la fenêtre
/// (sans panique) n'est jamais rejouée.
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

/// Variable posée sur le processus relancé en rendu logiciel : il ne se relance pas lui-même.
const FALLBACK_MARKER: &str = "BONK_RENDER_FALLBACK";

/// Le repli ne s'applique que si l'utilisateur n'a pas choisi lui-même le moteur de rendu
/// (`ICED_BACKEND`) et si on n'est pas déjà dans le processus de repli.
fn fallback_allowed(var: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    var("ICED_BACKEND").is_none() && var(FALLBACK_MARKER).is_none()
}

/// Ouvre la fenêtre en transformant une panique du rendu en erreur. wgpu traite certaines erreurs de
/// pilote comme fatales (panique sur le fil principal) au lieu de laisser Iced basculer sur le rendu
/// logiciel : sous un affichage virtuel, sur certaines machines virtuelles ou avec un pilote incomplet,
/// la surface ne peut pas être configurée (CDC §8 : le repli logiciel est obligatoire).
fn catching_panics(
    open: impl FnOnce() -> Result<(), String> + std::panic::UnwindSafe,
) -> Result<Result<(), String>, String> {
    let previous = std::panic::take_hook();
    // Le message est journalisé, pas imprimé : il serait redondant avec le repli, ou repris dans
    // l'erreur finale si le repli est impossible.
    std::panic::set_hook(Box::new(|info| {
        log::debug!(target: "bonk::gui", "panic in the window: {info}");
    }));
    let outcome = std::panic::catch_unwind(open);
    std::panic::set_hook(previous);
    outcome.map_err(|payload| panic_message(payload.as_ref()))
}

fn launch(initial: ViewOptions) -> Result<(), String> {
    iced::application(move || App::new(initial), App::update, App::view)
        .title("Bonk")
        .theme(App::theme)
        .subscription(App::subscription)
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
        .unwrap_or_else(|| "panique sans message".to_owned())
}

/// Relance Bonk avec les mêmes arguments et le moteur de rendu logiciel (`tiny-skia`), et attend sa
/// fin. Un nouveau processus plutôt qu'une variable posée ici : modifier l'environnement d'un
/// processus qui a déjà des fils n'est pas sûr, et l'état du rendu qui vient de paniquer n'est pas
/// réutilisable.
fn relaunch_with_software_renderer() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("repli impossible : {e}"))?;
    let status = std::process::Command::new(exe)
        .args(std::env::args_os().skip(1))
        .env("ICED_BACKEND", "tiny-skia")
        .env(FALLBACK_MARKER, "1")
        .status()
        .map_err(|e| format!("repli impossible : {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("le rendu logiciel a échoué aussi ({status})"))
    }
}

/// Sous Linux et les autres Unix hors macOS, une session graphique se reconnaît à ses variables
/// d'environnement ; ailleurs, le système fournit toujours un affichage.
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
        // L'utilisateur a choisi son moteur : on ne le contredit pas.
        assert!(!super::fallback_allowed(env(&["ICED_BACKEND"])));
        // Déjà dans le processus de repli : pas de boucle de relance.
        assert!(!super::fallback_allowed(env(&[super::FALLBACK_MARKER])));
    }

    #[test]
    fn message_d_une_panique() {
        let texte: Box<dyn std::any::Any + Send> = Box::new("wgpu error");
        let chaine: Box<dyn std::any::Any + Send> = Box::new(String::from("surface invalide"));
        let autre: Box<dyn std::any::Any + Send> = Box::new(42_u8);
        assert_eq!(super::panic_message(texte.as_ref()), "wgpu error");
        assert_eq!(super::panic_message(chaine.as_ref()), "surface invalide");
        assert_eq!(super::panic_message(autre.as_ref()), "panique sans message");
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
