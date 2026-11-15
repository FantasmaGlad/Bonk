use bonk_ui::model::{Model, WriteState};
use bonk_ui::text;
use serde_json::{Value, json};

mod menu;
mod panel;
mod vault;

use super::drives::disks_state;
use panel::panel_state;
pub(in crate::screen) use vault::vault_state;

#[must_use]
pub fn state(model: &Model) -> Value {
    let lang = model.lang;
    json!({
        "settings_button": lang.pick("Settings", "Paramètres"),
        "settings": crate::settings::state(model),
        "incognito": crate::incognito::state(model),
        "catalog_button": crate::browse::button(model),
        "browse": crate::browse::state(model),
        // The app reads its own texts (Android resources, notification) in this language.
        "lang": lang.pick("en", "fr"),
        "status": model.status_line(model.entries().len()),
        "notice": model.requested().and_then(|(id, requested)| {
            let disk_chosen = matches!(model.write, WriteState::Draft { .. });
            text::requested_line(lang, id, requested, disk_chosen)
        }),
        "empty": lang.pick(
            "No disk offered. Plug in a USB drive or an SD card with an adapter.",
            "Aucun disque proposé. Branchez une clé USB ou une carte SD avec un adaptateur.",
        ),
        "disks": disks_state(model),
        "panel": panel_state(model),
        "vault": vault_state(model),
        "running": matches!(model.write, WriteState::Running { .. }) || model.vault.is_busy(),
    })
}
