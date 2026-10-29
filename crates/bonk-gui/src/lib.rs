//! Interface graphique de Bonk (Iced) : pour l'instant, une fenêtre de détection des disques en
//! lecture seule.
//!
//! Ce crate et `bonk-cli` ne se connaissent pas : tous deux s'appuient sur les mêmes crates de base.
//! Le rendu utilise wgpu et bascule sur le moteur logiciel tiny-skia quand aucun pilote graphique
//! n'est utilisable (CDC §8).

mod app;
mod command;
mod theme;

pub use command::{ViewOptions, equivalent_commands};

use iced::Size;

use crate::app::App;

/// Ouvre la fenêtre et rend la main à la fermeture.
///
/// # Errors
/// Texte de l'erreur d'Iced quand la fenêtre ne peut pas s'ouvrir (pas d'affichage, pas de rendu).
pub fn run() -> Result<(), String> {
    // Iced ne renvoie pas d'erreur sans affichage : il panique. Détection de la fonctionnalité en
    // amont, pour que Bonk redirige proprement vers la ligne de commande (SSH, serveur).
    if !display_available(|name| std::env::var_os(name)) {
        return Err("aucun affichage graphique (ni WAYLAND_DISPLAY ni DISPLAY)".to_owned());
    }
    iced::application(App::new, App::update, App::view)
        .title("Bonk")
        .theme(App::theme)
        .subscription(App::subscription)
        .window_size(Size::new(960.0, 640.0))
        .centered()
        .run()
        .map_err(|erreur| erreur.to_string())
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
