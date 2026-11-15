use bonk_ui::model::{Model, WRITE_DELAY, WriteState};
use bonk_ui::text;
use bonk_ui::vault::VaultWords;
use serde_json::{Value, json};

use super::menu::menu_state;

pub(in crate::screen) fn panel_state(model: &Model) -> Value {
    let lang = model.lang;
    match &model.write {
        WriteState::Idle => Value::Null,
        WriteState::Draft { .. } => draft_state(model),
        WriteState::Running {
            phase,
            done,
            total,
            rate,
            ..
        } => {
            let (line, fraction) = text::progress(lang, *phase, *done, *total, *rate);
            json!({
                "kind": "running",
                "line": line,
                "fraction": fraction,
                "cancel": lang.pick("Cancel", "Annuler"),
            })
        }
        WriteState::Finished { outcome, .. } => json!({
            "kind": "finished",
            "message": text::outcome(lang, outcome),
            "close": lang.pick("Close", "Fermer"),
        }),
    }
}

fn draft_state(model: &Model) -> Value {
    let lang = model.lang;
    let WriteState::Draft {
        disk_id,
        image,
        hash,
        raw,
        mbr,
        opened,
    } = &model.write
    else {
        return Value::Null;
    };
    let elapsed = opened.elapsed();
    let empty = image.trim().is_empty();
    let disk = model.disks.iter().find(|disk| &disk.id == disk_id);
    json!({
        "kind": "draft",
        "title": lang.pick("WRITE AN IMAGE TO THIS DISK", "ÉCRIRE UNE IMAGE SUR CE DISQUE"),
        "summary": disk.map(|disk| bonk_ui::recap::disk_line(lang, disk)),
        "image_line": bonk_ui::recap::image_line(lang, &model.catalog, image),
        "warning": bonk_ui::recap::warning(lang),
        "menu": menu_state(model),
        "advanced": model.advanced,
        "advanced_button": if model.advanced {
            lang.pick("Hide the manual image and options", "Masquer l'image manuelle et les options")
        } else {
            lang.pick("Other image or options...", "Autre image ou options...")
        },
        "image": image,
        "image_hint": lang.pick("https:// address of the image", "Adresse https:// de l'image"),
        "pick": lang.pick("Choose the image...", "Choisir l'image..."),
        "hash": hash,
        "hash_hint": lang.pick("SHA-256 hash (required for an address)", "Empreinte SHA-256 (exigée pour une adresse)"),
        "raw": raw,
        "raw_label": lang.pick(
            "Write the bytes as they are, without refusing a non-bootable image",
            "Écrire les octets tels quels, sans refuser une image non amorçable",
        ),
        "mbr": mbr,
        "mbr_label": lang.pick(
            "Windows drive: MBR table, boots in BIOS and UEFI up to 2 TiB (unchecked: GPT, UEFI only)",
            "Clé Windows : table MBR, démarre en BIOS et en UEFI jusqu'à 2 Tio (décochée : GPT, UEFI seul)",
        ),
        "button": text::write_button(lang, elapsed, empty),
        "enabled": elapsed >= WRITE_DELAY && !empty,
        "vault_button": VaultWords::new(lang).open,
    })
}
