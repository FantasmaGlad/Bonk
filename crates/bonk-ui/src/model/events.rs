use super::*;

pub fn ui_event(message: &Message) -> Option<(&'static str, String)> {
    Some(match message {
        Message::ToggleInternal(on) => ("toggle_internal", on.to_string()),
        Message::ToggleVirtual(on) => ("toggle_virtual", on.to_string()),
        Message::ToggleAll(on) => ("toggle_all", on.to_string()),
        Message::Select(id) => ("select_disk", id.clone()),
        Message::ToggleDetails => ("toggle_details", String::new()),
        Message::Nav(nav) => ("key", format!("{nav:?}")),
        Message::Copy => ("click_copy", String::new()),
        // The event detail only carries the path length.
        Message::ImageInput(text) => (
            "type_image_path",
            format!("{} caractères", text.chars().count()),
        ),
        Message::HashInput(text) => ("type_hash", format!("{} caractères", text.chars().count())),
        Message::ToggleRaw(on) => ("toggle_raw", on.to_string()),
        Message::ToggleTor(on) => ("toggle_tor", on.to_string()),
        Message::TorDetected(found) => ("tor_detected", found.is_some().to_string()),
        Message::OpenTorProject => ("click_tor_project", String::new()),
        Message::ToggleMbr(on) => ("toggle_mbr", on.to_string()),
        Message::BrowseImage => ("click_browse", String::new()),
        // As for typing, the event detail does not carry the chosen path.
        Message::ImagePicked(path) => (
            "image_picked",
            if path.is_some() { "choisie" } else { "aucune" }.to_owned(),
        ),
        Message::ToggleMenu => ("click_menu", String::new()),
        Message::RequestImage(id) => ("open_link", id.clone()),
        Message::MenuCategory(value) | Message::MenuBase(value) | Message::MenuSystem(value) => {
            ("menu_filter", value.clone().unwrap_or_default())
        }
        Message::MenuChose(choice) => ("menu_chose", choice.label.clone()),
        Message::ToggleAdvanced => ("click_advanced", String::new()),
        Message::StartWrite => ("click_write", String::new()),
        Message::CancelWrite => ("click_cancel_write", String::new()),
        Message::DismissWrite => ("click_close_write", String::new()),
        Message::ConfirmInternal => ("click_confirm_internal", String::new()),
        Message::CloseInternal => ("click_close_internal", String::new()),
        Message::Vault(message) => return vault_event(message),
        Message::Settings(message) => return settings_event(*message),
        Message::Library(message) => return library_event(message),
        Message::Browse(message) => ("catalog_tab", format!("{message:?}")),
        Message::Tick
        | Message::Clock
        | Message::Loaded(_)
        | Message::Write(_)
        | Message::CatalogLoaded(_) => return None,
    })
}

pub(super) fn vault_event(message: &crate::vault::VaultMessage) -> Option<(&'static str, String)> {
    use crate::vault::VaultMessage;
    Some(match message {
        VaultMessage::Open => ("click_vault", String::new()),
        VaultMessage::Close => ("click_close_vault", String::new()),
        VaultMessage::Prepare => ("click_prepare_vault", String::new()),
        VaultMessage::ConfirmPrepare => ("click_confirm_vault", String::new()),
        VaultMessage::Browse => ("click_add_iso", String::new()),
        // As for the image, the event detail does not carry the chosen path.
        VaultMessage::Picked(path) => ("iso_picked", path.is_some().to_string()),
        VaultMessage::Update => ("click_update_vault", String::new()),
        VaultMessage::Cancel => ("click_cancel_vault", String::new()),
        VaultMessage::Event(_) => return None,
    })
}

pub(super) fn settings_event(
    message: crate::settings::SettingsMessage,
) -> Option<(&'static str, String)> {
    use crate::settings::SettingsMessage;
    Some(match message {
        SettingsMessage::Open => ("click_settings", String::new()),
        SettingsMessage::Close => ("click_close_settings", String::new()),
        SettingsMessage::Language(choice) => ("choose_language", choice.config_value().to_owned()),
        SettingsMessage::Theme(choice) => ("choose_theme", choice.config_value().to_owned()),
        SettingsMessage::Offline(on) => ("toggle_offline", on.to_string()),
    })
}

pub(super) fn library_event(
    message: &crate::library::LibraryMessage,
) -> Option<(&'static str, String)> {
    use crate::library::LibraryMessage;
    Some(match message {
        LibraryMessage::Open => ("click_library", String::new()),
        LibraryMessage::Close => ("click_close_library", String::new()),
        LibraryMessage::Remove(sha256) => ("click_remove_kept", sha256.clone()),
        LibraryMessage::Write(sha256) => ("click_write_kept", sha256.clone()),
        LibraryMessage::Listed(_) | LibraryMessage::Removed(_) => return None,
    })
}
