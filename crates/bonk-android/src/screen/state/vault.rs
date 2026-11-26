use bonk_core::write::Phase;
use bonk_hal::{Lang, format_size};
use bonk_ui::model::Model;
use bonk_ui::text;
use bonk_ui::vault::{VaultAction, VaultState, VaultWords};
use serde_json::{Value, json};

fn vault_button(label: &str, action: &str, primary: bool) -> Value {
    json!({"label": label, "action": action, "primary": primary})
}

/// Lines, progress fraction and buttons of a vault state.
type VaultBody = (Vec<String>, Option<f32>, Vec<Value>);

pub(in crate::screen) fn vault_state(model: &Model) -> Value {
    let lang = model.lang;
    let words = VaultWords::new(lang);
    let close = vault_button(lang.pick("Close", "Fermer"), "close", false);
    let mut images = Vec::new();
    let (lines, fraction, buttons): VaultBody = match &model.vault {
        VaultState::Closed => return Value::Null,
        VaultState::Busy {
            action,
            phase,
            done,
            total,
            rate,
            ..
        } => vault_busy(
            lang,
            *action,
            phase.map(|phase| (phase, *done, *total, *rate)),
        ),
        VaultState::NotVault { .. } if model.offline => (
            vec![
                words.not_vault.to_owned(),
                words.secure_boot.to_owned(),
                words.offline.to_owned(),
            ],
            None,
            vec![close],
        ),
        VaultState::NotVault { .. } => (
            vec![words.not_vault.to_owned(), words.secure_boot.to_owned()],
            None,
            vec![vault_button(words.prepare, "prepare", true), close],
        ),
        VaultState::Confirming { disk_id } => vault_confirming(model, &words, disk_id),
        VaultState::Ready { images: found, .. } => {
            images = found
                .iter()
                .map(|(name, size)| json!({"name": name, "size": format_size(*size, lang)}))
                .collect();
            vault_ready(&words, found.is_empty(), model.network_closed(), close)
        }
        VaultState::Finished { outcome, .. } => (
            vec![text::outcome(lang, outcome)],
            None,
            vec![vault_button("OK", "open", true)],
        ),
    };
    json!({
        "title": words.title,
        "busy": model.vault.is_busy(),
        "lines": lines,
        "fraction": fraction,
        "images": images,
        "buttons": buttons,
    })
}

/// `progress`: phase, bytes done, total, bytes per second; absent before the first event.
fn vault_busy(
    lang: Lang,
    action: VaultAction,
    progress: Option<(Phase, u64, Option<u64>, f64)>,
) -> VaultBody {
    let mut lines = vec![VaultWords::busy(lang, action).to_owned()];
    let mut fraction = None;
    if let Some((phase, done, total, rate)) = progress {
        let (line, part) = text::progress(lang, phase, done, total, rate);
        lines.push(line);
        fraction = Some(part);
    }
    let cancel = (action != VaultAction::List)
        .then(|| vault_button(lang.pick("Cancel", "Annuler"), "cancel", true));
    (lines, fraction, cancel.into_iter().collect())
}

fn vault_ready(words: &VaultWords, empty: bool, offline: bool, close: Value) -> VaultBody {
    let lines = [
        empty.then_some(words.empty),
        offline.then_some(words.offline),
    ]
    .into_iter()
    .flatten()
    .map(str::to_owned)
    .collect();
    let mut buttons = vec![vault_button(words.add, "browse", true)];
    if !offline {
        buttons.push(vault_button(words.update, "update", false));
    }
    buttons.push(close);
    (lines, None, buttons)
}

fn vault_confirming(model: &Model, words: &VaultWords, disk_id: &str) -> VaultBody {
    let lang = model.lang;
    let lines = model
        .disks
        .iter()
        .find(|disk| disk.id == disk_id)
        .map(|disk| text::disk_summary(lang, disk))
        .into_iter()
        .chain([lang
            .pick(
                "All the data on this disk will be erased. This cannot be undone.",
                "Toutes les données de ce disque seront effacées. Cette action est irréversible.",
            )
            .to_owned()])
        .collect();
    let buttons = vec![
        vault_button(words.erase, "confirm", true),
        vault_button(lang.pick("Cancel", "Annuler"), "open", false),
    ];
    (lines, None, buttons)
}
