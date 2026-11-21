use std::sync::atomic::Ordering;

use bonk_ui::browse::BrowseMessage;
use bonk_ui::event::Event;
use bonk_ui::model::{Effect, Message, Model, WriteState};
use bonk_ui::vault::VaultMessage;
use serde_json::{Value, json};

use super::*;

/// What Kotlin must do; what Rust can do alone does not leave here.
pub(super) fn effects_out(
    model: &mut Model,
    effects: Vec<Effect>,
    draft: Option<Value>,
) -> Vec<Value> {
    let mut out = Vec::new();
    for effect in effects {
        match effect {
            Effect::Enumerate => out.push(json!({"do": "enumerate"})),
            Effect::LoadCatalog { proxy } => out.push(json!({"do": "catalog", "proxy": proxy})),
            Effect::PickImage => out.push(json!({"do": "pick"})),
            Effect::SaveSetting { key, value } => {
                out.push(json!({"do": "save_setting", "key": key, "value": value}));
            }
            Effect::Relaunch { key, value } => {
                out.push(json!({"do": "relaunch", "key": key, "value": value}));
            }
            Effect::DetectTor | Effect::InspectImage(_) => in_rust(model, effect),
            Effect::CancelWrite(_) => CANCEL.store(true, Ordering::SeqCst),
            Effect::StartWrite(_) => {
                CANCEL.store(false, Ordering::SeqCst);
                let _ = model.update(Message::Write(Event::Started(0)));
                if let Some(mut write) = draft.clone() {
                    write["do"] = json!("write");
                    out.push(write);
                }
            }
            Effect::RunVault(args) => {
                CANCEL.store(false, Ordering::SeqCst);
                let _ = model.update(Message::Vault(VaultMessage::Event(Event::Started(0))));
                out.extend(vault_effect(model, &args));
            }
            Effect::PickVaultIso => out.push(json!({"do": "vault_pick"})),
            // No download cache on Android: the library is a desktop page.
            Effect::ListKept | Effect::RemoveKept(_) => {}
            Effect::OpenLink(url) => out.push(json!({"do": "open_link", "url": url})),
            Effect::RunDownload(args) => {
                let _ = model.update(Message::Browse(BrowseMessage::DownloadEvent(
                    Event::Started(0),
                )));
                out.extend(crate::browse::download::effect(model, &args));
            }
            Effect::Copy(_) | Effect::ScrollList(_) | Effect::ScrollMenu(_) => {}
        }
    }
    out
}

fn in_rust(model: &mut Model, effect: Effect) {
    let message = match effect {
        // Orbot listens on the phone's loopback, as Tor does on a computer.
        Effect::DetectTor => Message::TorDetected(bonk_catalog::tor::detect()),
        Effect::InspectImage(image) => {
            let report = bonk_ops::job::inspect(std::path::Path::new(&image))
                .ok()
                .and_then(|inspection| serde_json::to_value(inspection).ok());
            Message::Inspected(image, report)
        }
        _ => return,
    };
    let _ = model.update(message);
}

/// The vault command the model asks for, for Kotlin to run: the phone has no `bonk` command. Kotlin
/// downloads GRUB for `init` and `update`, one image per processor; Rust checks each against the signed
/// catalog.
fn vault_effect(model: &Model, args: &[String]) -> Option<Value> {
    let action = args.get(1)?.as_str();
    let disk_id = args.get(2)?;
    let device = model
        .disks
        .iter()
        .find(|disk| &disk.id == disk_id)
        .map(|disk| disk.path.clone());
    let iso = (action == "add").then(|| args.get(3)).flatten();
    let grub: Vec<Value> = if matches!(action, "init" | "update") {
        crate::vault::grub_downloads()
            .into_iter()
            .map(|image| json!({"arch": image.arch, "url": image.url, "size_bytes": image.size_bytes}))
            .collect()
    } else {
        Vec::new()
    };
    Some(json!({
        "do": "vault",
        "action": action,
        "disk_id": disk_id,
        "device": device,
        "iso": iso,
        "grub": grub,
        "proxy": model.proxy(),
    }))
}

pub(super) fn draft_of(model: &Model) -> Option<Value> {
    let WriteState::Draft {
        disk_id,
        image,
        hash,
        raw,
        mbr,
        ..
    } = &model.write
    else {
        return None;
    };
    let device = model
        .disks
        .iter()
        .find(|disk| &disk.id == disk_id)
        .map(|disk| disk.path.clone());
    Some(json!({
        "disk_id": disk_id,
        "device": device,
        "image": image.trim(),
        "hash": hash.trim(),
        "raw": raw,
        "mbr": mbr,
        "hybrid": model.catalog.hybrid_of(image.trim()),
        "proxy": model.proxy(),
    }))
}
