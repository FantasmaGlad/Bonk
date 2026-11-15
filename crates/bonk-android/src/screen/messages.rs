use bonk_core::write::Phase;
use bonk_ui::catalog::{CatalogState, writable};
use bonk_ui::event::{Event, Outcome};
use bonk_ui::model::{Message, Model};
use bonk_ui::vault::VaultMessage;
use serde_json::Value;

use super::drives::disk_of;

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
pub(super) fn drive_event(model: &Model, event: Event) -> Message {
    if model.vault.is_busy() {
        Message::Vault(VaultMessage::Event(event))
    } else {
        Message::Write(event)
    }
}

pub(super) fn message_of(input: &Value, model: &Model) -> Option<Message> {
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
        "incognito" | "tor_project" => crate::incognito::message_of(input, model)?,
        "menu" => Message::ToggleMenu,
        "category" => Message::MenuCategory(
            text_of("value").and_then(|shown| model.terms.uses().id_of(&shown, model.lang)),
        ),
        "base" => Message::MenuBase(
            text_of("value").and_then(|shown| model.terms.bases().id_of(&shown, model.lang)),
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
