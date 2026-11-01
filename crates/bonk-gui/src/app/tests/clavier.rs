//! Navigation au clavier.

use super::*;

pub(super) fn nav(app: &mut App, nav: Nav) {
    let _ = app.update(Message::Nav(nav));
}

pub(super) fn avec_tout_affiche() -> App {
    let mut app = loaded(sample());
    let _ = app.update(Message::ToggleAll(true));
    let _ = app.update(Message::ToggleInternal(true));
    app.focus = None;
    app
}

#[test]
fn tab_parcourt_les_commandes_puis_les_disques_et_reboucle() {
    let mut app = avec_tout_affiche();
    let mut vus = Vec::new();
    for _ in 0..10 {
        nav(&mut app, Nav::Next);
        vus.push(app.focus.clone());
    }
    assert_eq!(
        vus,
        [
            Some(Focus::Refresh),
            Some(Focus::Internal),
            Some(Focus::Virtual),
            Some(Focus::All),
            Some(Focus::Disk("nvme-systeme".to_owned())),
            Some(Focus::Disk("usb-cle".to_owned())),
            Some(Focus::Disk("sata-libre".to_owned())),
            Some(Focus::Copy),
            Some(Focus::Language),
            Some(Focus::Refresh),
        ]
    );
}

#[test]
fn maj_tab_remonte_et_commence_par_le_dernier() {
    let mut app = avec_tout_affiche();
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Language));
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Copy));
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
}

#[test]
fn espace_a_le_meme_effet_qu_un_clic() {
    let mut app = loaded(sample());
    app.focus = Some(Focus::Internal);
    nav(&mut app, Nav::Activate);
    assert!(app.show_internal);
    nav(&mut app, Nav::Activate);
    assert!(!app.show_internal);

    app.focus = Some(Focus::All);
    nav(&mut app, Nav::Activate);
    assert!(app.show_all);

    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    nav(&mut app, Nav::Activate);
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    nav(&mut app, Nav::Activate);
    assert_eq!(app.selected, None);

    app.focus = Some(Focus::Refresh);
    nav(&mut app, Nav::Activate);
    assert!(app.loading);
}

#[test]
fn rien_n_est_active_sans_focus() {
    let mut app = loaded(sample());
    nav(&mut app, Nav::Activate);
    assert!(!app.show_all && !app.show_internal && app.selected.is_none() && !app.loading);
}

#[test]
fn les_fleches_se_deplacent_entre_les_disques() {
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Refresh);
    // Bas depuis une commande entre dans la liste.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
    nav(&mut app, Nav::Down);
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Pas de bouclage en bas ni en haut.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    nav(&mut app, Nav::Up);
    nav(&mut app, Nav::Up);
    nav(&mut app, Nav::Up);
    assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
}

#[test]
fn echap_referme_le_detail_puis_retire_le_focus() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    nav(&mut app, Nav::Escape);
    assert_eq!(app.selected, None);
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
    nav(&mut app, Nav::Escape);
    assert_eq!(app.focus, None);
}

#[test]
fn le_focus_quitte_un_disque_qui_disparait() {
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Disk("sata-libre".to_owned()));
    // La case « écartés » masque le disque interne : le focus ne reste pas sur un composant absent.
    let _ = app.update(Message::ToggleInternal(false));
    assert_eq!(app.focus, Some(Focus::Internal));
    app.focus = Some(Focus::Disk("sata-libre".to_owned()));
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert_eq!(app.focus, None);
    // Un échec de détection retire aussi les disques de l'ordre de Tab.
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Err("panne".to_owned())));
    assert_eq!(app.focus, None);
    nav(&mut app, Nav::Next);
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Language));
}

#[test]
fn un_clic_donne_le_focus_au_composant() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
    let _ = app.update(Message::ToggleAll(true));
    assert_eq!(app.focus, Some(Focus::All));
    // Le minuteur ne touche pas au focus.
    let _ = app.update(Message::Tick);
    assert_eq!(app.focus, Some(Focus::All));
}

#[test]
fn touches_vers_actions() {
    use keyboard::Modifiers as M;
    let tab = Key::Named(Named::Tab);
    let nav_de = |key: &Key, mods: M, repeat: bool| match map_key(key, mods, repeat) {
        Some(Message::Nav(n)) => Some(n),
        _ => None,
    };
    assert_eq!(nav_de(&tab, M::empty(), false), Some(Nav::Next));
    assert_eq!(nav_de(&tab, M::SHIFT, false), Some(Nav::Previous));
    assert_eq!(
        nav_de(&Key::Named(Named::Space), M::empty(), false),
        Some(Nav::Activate)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::Enter), M::empty(), false),
        Some(Nav::Activate)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::ArrowDown), M::empty(), false),
        Some(Nav::Down)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::ArrowUp), M::empty(), false),
        Some(Nav::Up)
    );
    assert_eq!(
        nav_de(&Key::Named(Named::Escape), M::empty(), false),
        Some(Nav::Escape)
    );
    // Raccourcis du système laissés tranquilles, répétition de touche ignorée pour activer.
    assert_eq!(nav_de(&tab, M::CTRL, false), None);
    assert_eq!(nav_de(&tab, M::ALT, false), None);
    assert_eq!(nav_de(&Key::Named(Named::Space), M::empty(), true), None);
    assert_eq!(nav_de(&tab, M::empty(), true), Some(Nav::Next));
    assert_eq!(nav_de(&Key::Character("a".into()), M::empty(), false), None);
}

#[test]
fn haut_depuis_le_bouton_de_copie_revient_au_dernier_disque() {
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Copy);
    nav(&mut app, Nav::Up);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Bas depuis le dernier disque n'entre pas dans le bouton : c'est le rôle de Tab.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Sans disque affiché, Haut depuis la copie ne fait rien.
    let mut vide = loaded(vec![]);
    vide.focus = Some(Focus::Copy);
    nav(&mut vide, Nav::Up);
    assert_eq!(vide.focus, Some(Focus::Copy));
}
