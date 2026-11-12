use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use bonk_core::partition::Layout;
use bonk_core::write::Phase;
use bonk_hal::{Bus, Disk, DiskKind, Lang, Partition, format_size, state_label, verdict_badge};
use bonk_ui::catalog::{
    CatalogState, ChoiceSource, all_categories, all_systems, official_page_note, writable,
};
use bonk_ui::command::ViewOptions;
use bonk_ui::event::{Event, Outcome};
use bonk_ui::model::{Effect, Message, Model, WRITE_DELAY, WriteState};
use bonk_ui::text;
use bonk_ui::uses;
use bonk_ui::vault::{VaultAction, VaultMessage, VaultState, VaultWords};
use serde_json::{Value, json};

pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);
pub(crate) static EVENTS: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());
/// A panic while holding a lock poisons it: refusing access afterwards would leave the screen dead until the
/// app restarts. The state is taken as it was; a running write has its own checks (read-back, hash), and the
/// worst case is a screen to refresh.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) static MODEL: LazyLock<Mutex<Model>> =
    LazyLock::new(|| Mutex::new(Model::new(ViewOptions::default(), Lang::En).0));

/// As the safety filter judges it. Android only shows USB: neither the phone's internal storage nor its
/// partitions, which the system keeps.
fn disk_of(value: &Value) -> Option<Disk> {
    let serial = value["serial"].as_str().filter(|s| !s.is_empty());
    let name = value["device"].as_str()?;
    // The serial alone is not enough: cloned keys share it, and the tapped card would then name the first in
    // the list. The plug point tells them apart; replugging the key changes its id, and the draft starts over
    // (empty image, new delay) instead of targeting whatever took the place.
    let port = name
        .strip_prefix("/dev/bus/usb/")
        .unwrap_or(name)
        .replace('/', "-");
    Some(Disk {
        id: match serial {
            Some(serial) => format!("usb-{serial}-{port}"),
            None => format!("usb-{port}"),
        },
        location: Some(port.clone()),
        path: name.to_owned(),
        model: value["model"].as_str()?.to_owned(),
        serial: serial.map(str::to_owned),
        size_bytes: value["size_bytes"].as_u64()?,
        bus: Bus::Usb,
        kind: DiskKind::Physical,
        removable_flag: true,
        logical_block_size: None,
        fs_type: None,
        holders: Vec::new(),
        partitions: value["partitions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(partition_of)
            .collect(),
    })
}

/// Without them the summary before erasing would show a full key as empty. GPT first, else MBR without the
/// protective entry.
pub(crate) fn partitions_of(layout: &Layout) -> Vec<Value> {
    let block = u64::from(layout.logical_block_size);
    if let Some(gpt) = layout.gpt.as_ref().filter(|gpt| !gpt.entries.is_empty()) {
        return gpt
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "label": (!entry.name.is_empty()).then(|| entry.name.clone()),
                    "size_bytes": (entry.last_lba.saturating_sub(entry.first_lba) + 1).saturating_mul(block),
                })
            })
            .collect();
    }
    layout
        .mbr
        .iter()
        .filter(|mbr| mbr.signature_valid && !mbr.protective)
        .flat_map(|mbr| &mbr.entries)
        .filter(|entry| entry.kind != 0)
        .map(|entry| json!({"label": null, "size_bytes": u64::from(entry.sectors).saturating_mul(block)}))
        .collect()
}

/// As [`partitions_of`] reads them, in the shape the safety filter judges.
pub(crate) fn partitions(layout: &Layout) -> Vec<Partition> {
    partitions_of(layout)
        .iter()
        .enumerate()
        .filter_map(partition_of)
        .collect()
}

fn partition_of((index, value): (usize, &Value)) -> Option<Partition> {
    Some(Partition {
        path: format!("{}", index + 1),
        label: value["label"].as_str().map(str::to_owned),
        fs_type: None,
        size_bytes: value["size_bytes"].as_u64()?,
        mount_points: Vec::new(),
        holders: Vec::new(),
        is_system_volume: false,
        is_active_esp: false,
    })
}

fn vault_message(action: &str) -> Option<VaultMessage> {
    Some(match action {
        "open" => VaultMessage::Open,
        "close" => VaultMessage::Close,
        "prepare" => VaultMessage::Prepare,
        "confirm" => VaultMessage::ConfirmPrepare,
        "browse" => VaultMessage::Browse,
        "update" => VaultMessage::Update,
        "cancel" => VaultMessage::Cancel,
        _ => return None,
    })
}

/// What the engine or the app reports belongs to the drive work running: the vault's or the write's.
fn drive_event(model: &Model, event: Event) -> Message {
    if model.vault.is_busy() {
        Message::Vault(VaultMessage::Event(event))
    } else {
        Message::Write(event)
    }
}

fn message_of(input: &Value, model: &Model) -> Option<Message> {
    let flag = || input["on"].as_bool().unwrap_or(false);
    let text_of = |key: &str| input[key].as_str().map(str::to_owned);
    Some(match input["type"].as_str()? {
        "disks" => Message::Loaded(Ok(input["disks"]
            .as_array()?
            .iter()
            .filter_map(disk_of)
            .collect())),
        "tick" => Message::Tick,
        "clock" => Message::Clock,
        "select" => Message::Select(text_of("id")?),
        "details" => Message::ToggleDetails,
        "show_all" => Message::ToggleAll(flag()),
        "settings" => crate::settings::message_of(input)?,
        "menu" => Message::ToggleMenu,
        "category" => Message::MenuCategory(
            text_of("value").and_then(|shown| uses::id_of(&shown, model.lang)),
        ),
        "system" => Message::MenuSystem(text_of("value")),
        "catalog_failed" => Message::CatalogLoaded(Err(text_of("message")?)),
        "choose" => chosen(input, model)?,
        "advanced" => Message::ToggleAdvanced,
        "image" => Message::ImageInput(text_of("value")?),
        "hash" => Message::HashInput(text_of("value")?),
        "raw" => Message::ToggleRaw(flag()),
        "mbr" => Message::ToggleMbr(flag()),
        "browse" => Message::BrowseImage,
        "picked" => Message::ImagePicked(text_of("value").map(Into::into)),
        "start" => Message::StartWrite,
        "confirm" => Message::ConfirmInternal,
        "close_confirm" => Message::CloseInternal,
        "cancel" => Message::CancelWrite,
        "dismiss" => Message::DismissWrite,
        "download" | "failed" => drive_event(model, app_event(input)?),
        "vault" => Message::Vault(vault_message(input["action"].as_str()?)?),
        "vault_picked" => Message::Vault(VaultMessage::Picked(text_of("value").map(Into::into))),
        _ => return None,
    })
}

/// The writable menu row the user touched, by its position.
fn chosen(input: &Value, model: &Model) -> Option<Message> {
    let CatalogState::Ready(systems) = &model.catalog else {
        return None;
    };
    let index = usize::try_from(input["index"].as_u64()?).ok()?;
    let choice = writable(&model.menu_filter.apply(systems))
        .get(index)
        .copied()?;
    Some(Message::MenuChose(choice.clone()))
}

/// Download progress, or a failure born in the app (cancel, key gone, download): its code, when it has one,
/// shows the translated title, as for an engine error.
fn app_event(input: &Value) -> Option<Event> {
    Some(match input["type"].as_str()? {
        "download" => Event::Progress {
            phase: Phase::Download,
            done: input["done"].as_u64()?,
            total: input["total"].as_u64(),
            rate: input["rate"].as_f64().unwrap_or(0.0),
        },
        _ => Event::Finished(Outcome::Failed {
            code: input["code"].as_str().map(str::to_owned),
            message: input["message"].as_str()?.to_owned(),
        }),
    })
}

/// What Kotlin must do; what Rust can do alone does not leave here.
fn effects_out(model: &mut Model, effects: Vec<Effect>, draft: Option<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    for effect in effects {
        match effect {
            Effect::Enumerate => out.push(json!({"do": "enumerate"})),
            Effect::LoadCatalog => out.push(json!({"do": "catalog"})),
            Effect::PickImage => out.push(json!({"do": "pick"})),
            Effect::SaveSetting { key, value } => {
                out.push(json!({"do": "save_setting", "key": key, "value": value}));
            }
            Effect::Relaunch { key, value } => {
                out.push(json!({"do": "relaunch", "key": key, "value": value}));
            }
            // No Tor on the phone without a dedicated app: the option stays off.
            Effect::DetectTor => {
                let _ = model.update(Message::TorDetected(None));
            }
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
            Effect::Copy(_) | Effect::ScrollList(_) | Effect::ScrollMenu(_) => {}
        }
    }
    out
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
    }))
}

fn draft_of(model: &Model) -> Option<Value> {
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
    }))
}

fn disks_state(model: &Model) -> Vec<Value> {
    let lang = model.lang;
    model
        .entries()
        .iter()
        .map(|(disk, verdict)| {
            let selected = model.selected.as_deref() == Some(disk.id.as_str());
            json!({
                "id": disk.id,
                "device": disk.path,
                "model": disk.model,
                "facts": bonk_ui::disk::facts(lang, disk),
                "badge": verdict_badge(verdict, lang),
                "state": state_label(verdict, lang),
                "selected": selected,
                // Device node, serial and partitions: on demand, under the selected card.
                "details": if selected && model.details {
                    bonk_ui::disk::details(lang, disk)
                } else {
                    Vec::new()
                },
                "details_button": selected.then(|| bonk_ui::disk::details_button(lang, model.details)),
            })
        })
        .collect()
}

fn menu_state(model: &Model) -> Value {
    let lang = model.lang;
    let CatalogState::Ready(systems) = &model.catalog else {
        let line = match &model.catalog {
            CatalogState::Failed(error) => error.clone(),
            _ => lang
                .pick(
                    "Reading the list of systems...",
                    "Lecture de la liste des systèmes...",
                )
                .to_owned(),
        };
        return json!({"ready": false, "line": line});
    };
    let (categories, names) = model.menu_filter.lists(systems);
    let categories: Vec<String> = categories.iter().map(|id| uses::label(id, lang)).collect();
    let filtered = model.menu_filter.apply(systems);
    let mut index = 0;
    let mut rows = Vec::new();
    // Long list: building and rereading it at every tick would slow the whole screen for nothing.
    let shown: &[&bonk_ui::catalog::System] = if model.menu_open { &filtered } else { &[] };
    for system in shown {
        for choice in &system.choices {
            let usable = matches!(choice.source, ChoiceSource::Download { .. });
            let note = match &choice.source {
                ChoiceSource::OfficialPage(page) => Some(official_page_note(page, lang)),
                _ => choice.note.clone(),
            };
            rows.push(json!({
                "title": format!("{} {}", choice.name, choice.title(lang)),
                "note": note,
                "index": usable.then_some(index),
            }));
            index += usize::from(usable);
        }
    }
    json!({
        "ready": true,
        "open": model.menu_open,
        "button": chosen_title(model).unwrap_or_else(|| lang.pick("Choose a system...", "Choisir un système...").to_owned()),
        "all_categories": all_categories(lang),
        "all_systems": all_systems(lang),
        "category": model.menu_filter.category.as_deref().map(|id| uses::label(id, lang)),
        "system": model.menu_filter.system,
        "categories": categories,
        "systems": names,
        "rows": rows,
    })
}

fn chosen_title(model: &Model) -> Option<String> {
    let (CatalogState::Ready(systems), WriteState::Draft { image, .. }) =
        (&model.catalog, &model.write)
    else {
        return None;
    };
    bonk_ui::catalog::find_url(systems, image)
        .map(|(_, choice)| format!("{} {}", choice.name, choice.title(model.lang)))
}

fn vault_button(label: &str, action: &str, primary: bool) -> Value {
    json!({"label": label, "action": action, "primary": primary})
}

/// Lines, progress fraction and buttons of a vault state.
type VaultBody = (Vec<String>, Option<f32>, Vec<Value>);

fn vault_state(model: &Model) -> Value {
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
        VaultState::NotVault { .. } => (
            vec![words.not_vault.to_owned()],
            None,
            vec![vault_button(words.prepare, "prepare", true), close],
        ),
        VaultState::Confirming { disk_id } => vault_confirming(model, &words, disk_id),
        VaultState::Ready { images: found, .. } => {
            images = found
                .iter()
                .map(|(name, size)| json!({"name": name, "size": format_size(*size, lang)}))
                .collect();
            vault_ready(&words, found.is_empty(), close)
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

fn vault_ready(words: &VaultWords, empty: bool, close: Value) -> VaultBody {
    let lines = empty.then(|| words.empty.to_owned()).into_iter().collect();
    let buttons = vec![
        vault_button(words.add, "browse", true),
        vault_button(words.update, "update", false),
        close,
    ];
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

fn panel_state(model: &Model) -> Value {
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

#[must_use]
pub fn state(model: &Model) -> Value {
    let lang = model.lang;
    json!({
        "settings_button": lang.pick("Settings", "Paramètres"),
        "settings": crate::settings::state(model),
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

/// A link another app opened Bonk with. Only a catalog image name gets through: it chooses the image, and
/// the user still picks the drive and starts the write.
fn open_link(model: &mut Model, input: &Value) -> Vec<Effect> {
    if input["type"] != "open" {
        return Vec::new();
    }
    match input["link"].as_str().map(bonk_catalog::link::image_id) {
        Some(Ok(id)) => model.request_image(id.to_owned()),
        _ => Vec::new(),
    }
}

/// A message, or just a look at the state with `{}`: the state and the effects to perform.
#[must_use]
pub fn dispatch(input: &str) -> String {
    let mut model = lock(&MODEL);
    let input: Value = serde_json::from_str(input).unwrap_or_default();
    let mut effects = Vec::new();
    let pending: Vec<Event> = lock(&EVENTS).drain(..).collect();
    for event in pending {
        let message = drive_event(&model, event);
        effects.extend(model.update(message));
    }
    let draft = draft_of(&model);
    if let Some(message) = message_of(&input, &model) {
        effects.extend(model.update(message));
    }
    effects.extend(open_link(&mut model, &input));
    let effects = effects_out(&mut model, effects, draft);
    json!({"state": state(&model), "effects": effects}).to_string()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn usb() -> Value {
        json!({"device": "/dev/bus/usb/001/002", "model": "UDisk", "serial": "S1", "size_bytes": 32_000_000_000_u64})
    }

    fn loaded() -> Model {
        let mut model = Model {
            auto_select: true,
            ..Model::default()
        };
        let disks: Vec<Disk> = disk_of(&usb()).into_iter().collect();
        let _ = model.update(Message::Loaded(Ok(disks)));
        model
    }

    #[test]
    fn la_cle_passe_le_meme_filtre_que_sur_pc_et_se_selectionne() {
        let model = loaded();
        assert_eq!(model.selected.as_deref(), Some("usb-S1-001-002"));
        let state = state(&model);
        assert_eq!(state["disks"][0]["model"], "UDisk");
        assert_eq!(state["panel"]["kind"], "draft");
    }

    #[test]
    fn le_bouton_attend_l_image_et_les_trois_secondes() {
        let mut model = loaded();
        assert_eq!(state(&model)["panel"]["enabled"], false);
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        assert_eq!(
            state(&model)["panel"]["enabled"],
            false,
            "délai de réflexion"
        );
        if let WriteState::Draft { opened, .. } = &mut model.write {
            *opened = Instant::now()
                .checked_sub(Duration::from_secs(4))
                .unwrap_or(*opened);
        }
        assert_eq!(state(&model)["panel"]["enabled"], true);
    }

    #[test]
    fn un_depart_demande_a_kotlin_d_ecrire_avec_le_brouillon() {
        let mut model = loaded();
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        if let WriteState::Draft { opened, .. } = &mut model.write {
            *opened = Instant::now()
                .checked_sub(Duration::from_secs(4))
                .unwrap_or(*opened);
        }
        let draft = draft_of(&model);
        let effects = model.update(Message::StartWrite);
        let out = effects_out(&mut model, effects, draft);
        assert_eq!(out[0]["do"], "write");
        assert_eq!(out[0]["device"], "/dev/bus/usb/001/002");
        assert!(matches!(model.write, WriteState::Running { .. }));
    }

    /// The phone runs the vault through Kotlin, which opens the key; what comes back fills the vault panel,
    /// never the write panel.
    #[test]
    fn le_coffre_passe_par_kotlin_et_ses_retours_vont_au_coffre()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut model = loaded();
        let message = message_of(&json!({"type": "vault", "action": "open"}), &model);
        let effects = model.update(message.ok_or("message du coffre refusé")?);
        let out = effects_out(&mut model, effects, None);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["do"], "vault");
        assert_eq!(out[0]["action"], "list");
        assert_eq!(out[0]["device"], "/dev/bus/usb/001/002");
        assert!(model.vault.is_busy());
        assert!(matches!(model.write, WriteState::Draft { .. }));
        let failed = json!({"type": "failed", "message": "the drive is no longer connected", "code": "E1003"});
        let _ = model.update(message_of(&failed, &model).ok_or("échec de l'application refusé")?);
        assert!(matches!(
            &model.vault,
            VaultState::Finished { outcome: Outcome::Failed { code: Some(code), .. }, .. } if code == "E1003"
        ));
        assert!(matches!(model.write, WriteState::Draft { .. }));
        assert_eq!(state(&model)["vault"]["buttons"][0]["action"], "open");
        Ok(())
    }

    /// A row that changes height at each detection makes the whole screen jump.
    #[test]
    fn la_detection_periodique_ne_change_pas_l_etat_affiche() {
        let mut model = Model {
            auto_select: true,
            ..Model::default()
        };
        let _ = model.update(Message::Loaded(Ok(Vec::new())));
        let before = state(&model);
        let _ = model.update(Message::Tick);
        assert!(model.loading);
        assert_eq!(state(&model), before);
        let _ = model.update(Message::Loaded(Ok(Vec::new())));
        assert_eq!(state(&model), before);
    }

    /// Cloned keys often share a serial: each must keep its own card, and tapping the second must write the
    /// second, never the first in the list.
    #[test]
    fn deux_cles_de_meme_serie_restent_distinctes() {
        let mut model = Model {
            auto_select: false,
            ..Model::default()
        };
        let mut b = usb();
        b["device"] = json!("/dev/bus/usb/001/003");
        let disks: Vec<Disk> = [usb(), b].iter().filter_map(disk_of).collect();
        let _ = model.update(Message::Loaded(Ok(disks)));
        let shown = disks_state(&model);
        assert_eq!(shown.len(), 2);
        assert_ne!(shown[0]["id"], shown[1]["id"]);
        let second = shown
            .iter()
            .find(|disk| disk["device"] == "/dev/bus/usb/001/003")
            .and_then(|disk| disk["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_default();
        let _ = model.update(Message::Select(second));
        let selected: Vec<Value> = disks_state(&model)
            .into_iter()
            .filter(|disk| disk["selected"] == true)
            .map(|disk| disk["device"].clone())
            .collect();
        assert_eq!(selected, vec![json!("/dev/bus/usb/001/003")]);
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        assert_eq!(
            draft_of(&model).map(|draft| draft["device"].clone()),
            Some(json!("/dev/bus/usb/001/003"))
        );
    }

    /// The summary before erasing says what the key holds: a table read, never 0 by default.
    #[test]
    fn les_partitions_lues_sur_la_cle_arrivent_a_l_ecran() -> Result<(), Box<dyn std::error::Error>>
    {
        use bonk_core::MemoryDevice;
        use bonk_core::format::table::{TableKind, write_table};
        use bonk_core::format::{Filesystem, Ids};

        let mut dev = MemoryDevice::new(64 << 20, 512)?;
        write_table(
            &mut dev,
            TableKind::Gpt,
            Filesystem::Fat32,
            Some("DONNEES"),
            &Ids::from_seed(b"cle"),
            false,
        )?;
        let partitions = partitions_of(&bonk_core::partition::inspect(&dev)?);
        assert_eq!(partitions.len(), 1);
        let mut value = usb();
        value["partitions"] = json!(partitions);
        let mut model = Model {
            auto_select: true,
            ..Model::default()
        };
        let _ = model.update(Message::Loaded(Ok(disk_of(&value).into_iter().collect())));
        // The partition read on the key reaches the screen: its label is on the card.
        let facts = state(&model)["disks"][0]["facts"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(facts.contains("DONNEES"), "{facts}");
        // A blank key really has no partition.
        let blank = MemoryDevice::new(64 << 20, 512)?;
        assert!(partitions_of(&bonk_core::partition::inspect(&blank)?).is_empty());
        Ok(())
    }

    /// Every shape of the screen state: idle, draft with the menu open, writing, result.
    fn state_samples() -> Vec<Value> {
        use bonk_ui::catalog::{Choice, System};
        use bonk_ui::event::VaultDone;

        let mut idle = Model::default();
        let _ = open_link(
            &mut idle,
            &json!({"type": "open", "link": "bonk://install/debian-13-netinst-x86_64"}),
        );
        let mut samples = vec![state(&idle)];
        let mut model = loaded();
        model.catalog = CatalogState::Ready(vec![System {
            name: "Debian".to_owned(),
            uses: vec!["desktop".to_owned(), "server".to_owned()],
            icon: None,
            choices: vec![
                Choice {
                    id: "debian-13-netinst-x86_64".to_owned(),
                    name: "Debian".to_owned(),
                    label: "13.7.0 netinst".to_owned(),
                    end_of_life: false,
                    note: None,
                    source: ChoiceSource::Download {
                        url: "https://cdimage.debian.org/x.iso".to_owned(),
                        sha256: "ab".repeat(32),
                    },
                },
                Choice {
                    id: "debian-13-dvd-x86_64".to_owned(),
                    name: "Debian".to_owned(),
                    label: "DVD".to_owned(),
                    end_of_life: false,
                    note: Some("évaluation".to_owned()),
                    source: ChoiceSource::OfficialPage("https://www.debian.org/".to_owned()),
                },
            ],
        }]);
        let _ = model.update(Message::ToggleMenu);
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        // The button label counts the seconds to think: the sample wants them elapsed.
        if let WriteState::Draft { opened, .. } = &mut model.write {
            *opened = Instant::now()
                .checked_sub(Duration::from_secs(4))
                .unwrap_or(*opened);
        }
        samples.push(state(&model));
        let _ = model.update(Message::StartWrite);
        let _ = model.update(Message::Write(Event::Started(0)));
        let _ = model.update(Message::Write(Event::Progress {
            phase: Phase::Write,
            done: 1 << 20,
            total: Some(4 << 20),
            rate: 1_048_576.0,
        }));
        samples.push(state(&model));
        let _ = model.update(Message::Write(Event::Finished(Outcome::Failed {
            code: Some("E2002".to_owned()),
            message: "the drive changed".to_owned(),
        })));
        samples.push(state(&model));
        let _ = model.update(Message::DismissWrite);
        let _ = model.update(Message::Select("usb-S1-001-002".to_owned()));
        let _ = model.update(Message::Vault(VaultMessage::Open));
        let _ = model.update(Message::Vault(VaultMessage::Event(Event::Finished(
            Outcome::Vault(VaultDone::Listed(vec![(
                "debian-live.iso".to_owned(),
                1_500_000_000,
            )])),
        ))));
        samples.push(state(&model));
        let _ = model.update(Message::Vault(VaultMessage::Picked(Some(
            "/proc/self/fd/43".into(),
        ))));
        let _ = model.update(Message::Vault(VaultMessage::Event(Event::Progress {
            phase: Phase::Write,
            done: 1 << 20,
            total: Some(4 << 20),
            rate: 1_048_576.0,
        })));
        samples.push(state(&model));
        samples
    }

    /// The contract between Rust and the app, written to a file the Kotlin tests read back: effects as Rust
    /// emits them, messages as it accepts them. A change on one side breaks the other in CI.
    /// `BONK_UPDATE_PROTOCOL=1 cargo test -p bonk-android` rewrites the file after an intended change.
    #[test]
    fn le_protocole_avec_kotlin_suit_ses_echantillons() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = loaded();
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        let draft = draft_of(&model);
        let effects = effects_out(
            &mut model,
            vec![
                Effect::Enumerate,
                Effect::LoadCatalog,
                Effect::PickImage,
                Effect::SaveSetting {
                    key: "view.show_all",
                    value: "true",
                },
                Effect::Relaunch {
                    key: "theme",
                    value: "dark",
                },
                Effect::StartWrite(Vec::new()),
                Effect::PickVaultIso,
                Effect::RunVault(bonk_ui::vault::vault_args(
                    VaultAction::Add,
                    "usb-S1-001-002",
                    Some("/proc/self/fd/43"),
                    false,
                    false,
                )),
                Effect::RunVault(bonk_ui::vault::vault_args(
                    VaultAction::Update,
                    "usb-S1-001-002",
                    None,
                    false,
                    false,
                )),
            ],
            draft,
        );
        let messages = vec![
            json!({"type": "disks", "disks": [{
                "device": "/dev/bus/usb/001/002", "model": "Generic UDisk", "serial": "S1",
                "size_bytes": 32_000_000_000_u64,
                "partitions": [{"label": "ESP", "size_bytes": 1_048_576}, {"label": null, "size_bytes": 2_097_152}],
            }]}),
            json!({"type": "tick"}),
            json!({"type": "clock"}),
            json!({"type": "select", "id": "usb-S1-001-002"}),
            json!({"type": "details"}),
            json!({"type": "show_all", "on": true}),
            json!({"type": "settings", "action": "open"}),
            json!({"type": "settings", "action": "close"}),
            json!({"type": "settings", "action": "language", "value": "fr"}),
            json!({"type": "settings", "action": "theme", "value": "dark"}),
            json!({"type": "menu"}),
            json!({"type": "category", "value": "Servers"}),
            json!({"type": "category"}),
            json!({"type": "system", "value": "debian"}),
            json!({"type": "choose", "index": 0}),
            json!({"type": "advanced"}),
            json!({"type": "image", "value": "/x.iso"}),
            json!({"type": "hash", "value": "abc"}),
            json!({"type": "raw", "on": true}),
            json!({"type": "mbr", "on": false}),
            json!({"type": "browse"}),
            json!({"type": "picked", "value": "/proc/self/fd/42"}),
            json!({"type": "start"}),
            json!({"type": "cancel"}),
            json!({"type": "dismiss"}),
            json!({"type": "download", "done": 1024, "total": 4096, "rate": 512.0}),
            json!({"type": "failed", "message": "the drive is no longer connected", "code": "E1003"}),
            json!({"type": "failed", "message": "unexpected"}),
            json!({"type": "catalog_failed", "message": "404"}),
            json!({"type": "open", "link": "bonk://install/debian-13-netinst-x86_64"}),
            json!({"type": "vault", "action": "open"}),
            json!({"type": "vault", "action": "close"}),
            json!({"type": "vault", "action": "prepare"}),
            json!({"type": "vault", "action": "confirm"}),
            json!({"type": "vault", "action": "browse"}),
            json!({"type": "vault", "action": "update"}),
            json!({"type": "vault", "action": "cancel"}),
            json!({"type": "vault_picked", "value": "/proc/self/fd/43"}),
        ];
        // `choose` only means something with a loaded catalog, and `open` is not a screen message: their
        // samples fix the shape only.
        for message in messages
            .iter()
            .filter(|m| m["type"] != "choose" && m["type"] != "open")
        {
            assert!(
                message_of(message, &model).is_some(),
                "refusé par Rust : {message}"
            );
        }
        let states = state_samples();
        let text = serde_json::to_string_pretty(
            &json!({"effects": effects, "messages": messages, "states": states}),
        )? + "\n";
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../android/app/src/test/resources/protocole.json");
        if std::env::var_os("BONK_UPDATE_PROTOCOL").is_some() {
            std::fs::create_dir_all(path.parent().ok_or("chemin sans dossier")?)?;
            std::fs::write(&path, &text)?;
        }
        let written = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            written == text,
            "{} n'est plus à jour : BONK_UPDATE_PROTOCOL=1 cargo test -p bonk-android",
            path.display()
        );
        Ok(())
    }

    #[test]
    fn un_lien_choisit_une_image_du_catalogue_sans_rien_ecrire() {
        let mut model = loaded();
        let link = json!({"type": "open", "link": "bonk://install/debian-13-netinst-x86_64"});
        // The open draft already asked for the catalog: the link starts no second read.
        assert!(open_link(&mut model, &link).is_empty());
        assert_eq!(
            open_link(&mut Model::default(), &link),
            vec![Effect::LoadCatalog, Effect::DetectTor]
        );
        assert_eq!(model.requested.as_deref(), Some("debian-13-netinst-x86_64"));
        assert!(matches!(model.write, WriteState::Draft { .. }));
        for refused in [
            json!({"type": "open", "link": "bonk://install/x?disk=usb-S1"}),
            json!({"type": "open", "link": "https://example.org/x.iso"}),
            json!({"type": "open"}),
            json!({"type": "tick", "link": "bonk://install/other"}),
        ] {
            assert!(open_link(&mut model, &refused).is_empty(), "{refused}");
            assert_eq!(model.requested.as_deref(), Some("debian-13-netinst-x86_64"));
        }
    }

    /// A failure born in the app (cancel, key gone) carries a Bonk code: the screen shows its translated
    /// title and code, as for an engine error.
    #[test]
    fn un_echec_de_l_application_porte_son_code() -> Result<(), Box<dyn std::error::Error>> {
        let mut model = loaded();
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        if let WriteState::Draft { opened, .. } = &mut model.write {
            *opened = Instant::now()
                .checked_sub(Duration::from_secs(4))
                .unwrap_or(*opened);
        }
        let _ = model.update(Message::StartWrite);
        let _ = model.update(Message::Write(Event::Started(0)));
        let failed = json!({"type": "failed", "code": "E4003", "message": "cancelled"});
        let message = message_of(&failed, &model).ok_or("message refusé")?;
        let _ = model.update(message);
        let shown = state(&model)["panel"]["message"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(shown.contains("[E4003]"), "{shown}");
        Ok(())
    }

    /// A panic while holding the state must not leave the screen dead: the poisoned lock is taken back with
    /// the state as it was, instead of answering unavailable to every later message.
    #[test]
    fn un_verrou_empoisonne_est_repris() {
        let etat = std::sync::Arc::new(Mutex::new(1));
        let partage = std::sync::Arc::clone(&etat);
        let _ = std::thread::spawn(move || {
            let _garde = partage.lock();
            panic!("panique pendant la mise à jour");
        })
        .join();
        assert!(etat.is_poisoned());
        assert_eq!(*lock(&etat), 1);
    }

    #[test]
    fn l_etat_dit_la_langue_choisie() {
        let mut model = loaded();
        model.lang = Lang::Fr;
        assert_eq!(state(&model)["lang"], "fr");
        // A new language comes by a relaunch, never while the screen runs.
        let input = json!({"type": "settings", "action": "language", "value": "en"});
        let effects = model.update(message_of(&input, &model).unwrap_or(Message::Tick));
        assert_eq!(
            effects_out(&mut model, effects, None),
            [json!({"do": "relaunch", "key": "lang", "value": "en"})]
        );
        assert_eq!(state(&model)["lang"], "fr");
    }

    #[test]
    fn une_cle_de_taille_inconnue_n_est_pas_un_disque() {
        assert!(disk_of(&json!({"device": "d", "model": "m"})).is_none());
    }
}
