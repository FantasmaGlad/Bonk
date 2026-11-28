use bonk_core::write::Phase;
use bonk_ui::browse::BrowseMessage;
use bonk_ui::catalog::CatalogState;
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
    if crate::browse::download::running(model) {
        Message::Browse(BrowseMessage::DownloadEvent(event))
    } else if model.vault.is_busy() {
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
        "catalog" | "catalog_search" | "catalog_system" | "catalog_write" | "catalog_download"
        | "catalog_filter" => crate::browse::message_of(input, model)?,
        "menu" => Message::ToggleMenu,
        "category" => Message::MenuCategory(
            text_of("value").and_then(|shown| model.terms.uses().id_of(&shown, model.lang)),
        ),
        "base" => Message::MenuBase(
            text_of("value").and_then(|shown| model.terms.bases().id_of(&shown, model.lang)),
        ),
        "system" => Message::MenuSystem(text_of("value")),
        "menu_search" => Message::MenuSearch(text_of("value").unwrap_or_default()),
        "arch" => Message::MenuArch(text_of("value").and_then(|shown| arch_of(model, &shown))),
        "catalog_failed" => Message::CatalogLoaded(Err(text_of("message")?)),
        "choose" => chosen(input, model)?,
        "advanced" => Message::ToggleAdvanced,
        "image" => Message::ImageInput(text_of("value")?),
        "hash" => Message::HashInput(text_of("value")?),
        "raw" => Message::ToggleRaw(flag()),
        "mbr" => Message::ToggleMbr(flag()),
        "browse" => Message::BrowseImage,
        "picked" => Message::ImagePicked(text_of("value").map(Into::into)),
        "start" if crate::browse::download::running(model) => return None,
        "start" => Message::StartWrite,
        "confirm" => Message::ConfirmInternal,
        "close_confirm" => Message::CloseInternal,
        "cancel" => Message::CancelWrite,
        "dismiss" => Message::DismissWrite,
        "download" | "failed" => drive_event(model, app_event(input)?),
        "downloaded" => crate::browse::download::saved(input)?,
        "vault" if starts_vault_work(input) && crate::browse::download::running(model) => {
            return None;
        }
        "vault" => Message::Vault(vault_message(input["action"].as_str()?)?),
        "vault_picked" => Message::Vault(VaultMessage::Picked(text_of("value").map(Into::into))),
        _ => return None,
    })
}

/// A catalog image the server refused, explained as the command line explains it.
fn refusal(input: &Value) -> Option<String> {
    let failure = bonk_catalog::DownloadError::Refused {
        url: input["url"].as_str()?.to_owned(),
        status: u16::try_from(input["status"].as_u64()?).ok()?,
    };
    Some(failure.of_catalog_image())
}

fn starts_vault_work(input: &Value) -> bool {
    matches!(
        input["action"].as_str(),
        Some("confirm" | "browse" | "update")
    )
}

/// The processor whose label the app shows: labels are translated, the filter keeps the identifier.
fn arch_of(model: &Model, shown: &str) -> Option<String> {
    let CatalogState::Ready(systems) = &model.catalog else {
        return None;
    };
    systems
        .iter()
        .flat_map(|system| &system.choices)
        .map(|choice| choice.arch.as_str())
        .find(|arch| bonk_ui::catalog::arch_label(arch, model.lang) == shown)
        .map(str::to_owned)
}

fn chosen(input: &Value, model: &Model) -> Option<Message> {
    let CatalogState::Ready(systems) = &model.catalog else {
        return None;
    };
    let index = usize::try_from(input["index"].as_u64()?).ok()?;
    let choice = model.menu_filter.writable(systems).get(index).copied()?;
    Some(Message::MenuChose(Box::new(choice.clone())))
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
            message: refusal(input).unwrap_or(input["message"].as_str()?.to_owned()),
        }),
    })
}
