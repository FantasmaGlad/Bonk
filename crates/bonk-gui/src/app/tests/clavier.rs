use super::*;
use bonk_ui::settings::SettingsMessage;

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
    for _ in 0..12 {
        nav(&mut app, Nav::Next);
        vus.push(app.focus.clone());
    }
    assert_eq!(
        vus,
        [
            Some(Focus::Library),
            Some(Focus::Settings),
            Some(Focus::Disk("nvme-systeme".to_owned())),
            Some(Focus::Disk("usb-cle".to_owned())),
            Some(Focus::Disk("sata-libre".to_owned())),
            Some(Focus::Copy),
            Some(Focus::Library),
            Some(Focus::Settings),
            Some(Focus::Disk("nvme-systeme".to_owned())),
            Some(Focus::Disk("usb-cle".to_owned())),
            Some(Focus::Disk("sata-libre".to_owned())),
            Some(Focus::Copy),
        ]
    );
}

#[test]
fn maj_tab_remonte_et_commence_par_le_dernier() {
    let mut app = avec_tout_affiche();
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
    // The expert boxes live on the settings page.
    let _ = app.update(Message::Settings(SettingsMessage::Open));
    app.focus = Some(Focus::Internal);
    nav(&mut app, Nav::Activate);
    assert!(app.show_internal);
    nav(&mut app, Nav::Activate);
    assert!(!app.show_internal);

    app.focus = Some(Focus::All);
    nav(&mut app, Nav::Activate);
    assert!(app.show_all);

    let _ = app.update(Message::Settings(SettingsMessage::Close));
    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    nav(&mut app, Nav::Activate);
    assert_eq!(app.selected.as_deref(), Some("usb-cle"));
    nav(&mut app, Nav::Activate);
    assert_eq!(app.selected, None);

    app.focus = Some(Focus::Settings);
    nav(&mut app, Nav::Activate);
    assert_eq!(app.screen, bonk_ui::settings::Screen::Settings);
    // Escape goes back.
    nav(&mut app, Nav::Escape);
    assert_eq!(app.screen, bonk_ui::settings::Screen::Main);
    assert_eq!(app.focus, Some(Focus::Settings));
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
    app.focus = Some(Focus::Settings);
    // Down from a control enters the list.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("nvme-systeme".to_owned())));
    nav(&mut app, Nav::Down);
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // No wrap at the bottom or top.
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
    // Unticking the box hides the internal disk: focus does not stay on a missing disk.
    let _ = app.update(Message::Settings(SettingsMessage::Open));
    let _ = app.update(Message::ToggleInternal(false));
    assert_eq!(app.focus, Some(Focus::Internal));
    let _ = app.update(Message::Settings(SettingsMessage::Close));
    app.focus = Some(Focus::Disk("sata-libre".to_owned()));
    let _ = app.update(Message::Loaded(Ok(vec![])));
    assert_eq!(app.focus, None);
    // A detection failure also removes the disks from the Tab order.
    let mut app = avec_tout_affiche();
    app.focus = Some(Focus::Disk("usb-cle".to_owned()));
    let _ = app.update(Message::Loaded(Err("panne".to_owned())));
    assert_eq!(app.focus, None);
    nav(&mut app, Nav::Next);
    nav(&mut app, Nav::Previous);
    assert_eq!(app.focus, Some(Focus::Copy));
}

#[test]
fn un_clic_donne_le_focus_au_composant() {
    let mut app = loaded(sample());
    let _ = app.update(Message::Select("usb-cle".to_owned()));
    assert_eq!(app.focus, Some(Focus::Disk("usb-cle".to_owned())));
    let _ = app.update(Message::Settings(SettingsMessage::Open));
    let _ = app.update(Message::ToggleAll(true));
    assert_eq!(app.focus, Some(Focus::All));
    // The timer does not touch the focus.
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
    // System shortcuts left alone, key repeat ignored for activation.
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
    // Down from the last disk does not enter the button: that is Tab's job.
    nav(&mut app, Nav::Down);
    assert_eq!(app.focus, Some(Focus::Disk("sata-libre".to_owned())));
    // Without a disk shown, Up from the copy button does nothing.
    let mut vide = loaded(vec![]);
    vide.focus = Some(Focus::Copy);
    nav(&mut vide, Nav::Up);
    assert_eq!(vide.focus, Some(Focus::Copy));
}
