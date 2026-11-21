use bonk_hal::{Lang, format_size};
use bonk_ui::browse::{BrowseMessage, DownloadState};
use bonk_ui::catalog::{ChoiceSource, System};
use bonk_ui::event::{Event, Outcome};
use bonk_ui::model::{Message, Model, WriteState};
use bonk_ui::text;
use serde_json::{Value, json};

/// The download panel of the system page; `null` when none concerns this system.
pub(super) fn panel(model: &Model, system: &System) -> Value {
    let Some(download) = &model.browse.download else {
        return Value::Null;
    };
    let Some(choice) = system.choices.iter().find(|c| c.id == download.id()) else {
        return Value::Null;
    };
    let lang = model.lang;
    let title = format!("{} {}", choice.name, choice.title(lang));
    match download {
        DownloadState::Running { step, .. } => {
            let (line, fraction) = step.map_or_else(
                || {
                    let line = lang.pick("Starting the download...", "Début du téléchargement...");
                    (line.to_owned(), None)
                },
                |s| {
                    let (line, fraction) = text::progress(lang, s.phase, s.done, s.total, s.rate);
                    (line, s.total.map(|_| fraction))
                },
            );
            json!({
                "title": title,
                "line": line,
                "fraction": fraction,
                "button": lang.pick("Cancel", "Annuler"),
                "action": "cancel",
            })
        }
        DownloadState::Finished { outcome, .. } => json!({
            "title": title,
            "line": outcome_line(lang, outcome),
            "button": lang.pick("Close", "Fermer"),
            "action": "dismiss",
        }),
    }
}

/// The phone has no library: the app says where in its storage the image went.
fn outcome_line(lang: Lang, outcome: &Outcome) -> String {
    match outcome {
        Outcome::Downloaded { path, bytes } => format!(
            "{} {path} ({}).",
            lang.pick(
                "Image downloaded and checked, saved in",
                "Image téléchargée et contrôlée, enregistrée dans"
            ),
            format_size(*bytes, lang)
        ),
        other => text::outcome(lang, other),
    }
}

/// What Kotlin downloads: the address and hash come from the signed catalog, never from the app.
pub(crate) fn effect(model: &Model, args: &[String]) -> Option<Value> {
    let id = args.get(2)?;
    let choice = model
        .opened_system()?
        .choices
        .iter()
        .find(|choice| &choice.id == id)?;
    let ChoiceSource::Download {
        url,
        sha256,
        file_name,
        ..
    } = &choice.source
    else {
        return None;
    };
    Some(json!({
        "do": "download_image",
        "url": url,
        "sha256": sha256,
        "name": bonk_catalog::download_name(url, file_name.as_deref())?,
        "proxy": model.proxy(),
    }))
}

/// The app reports progress without saying for which task: a download waits for the drive to be free.
pub(super) fn message_of(input: &Value, model: &Model) -> Option<BrowseMessage> {
    Some(match input["action"].as_str()? {
        "start" if drive_busy(model) => return None,
        "start" => BrowseMessage::Download(input["id"].as_str()?.to_owned()),
        "cancel" => BrowseMessage::CancelDownload,
        "dismiss" => BrowseMessage::DismissDownload,
        _ => return None,
    })
}

pub(crate) fn running(model: &Model) -> bool {
    model
        .browse
        .download
        .as_ref()
        .is_some_and(DownloadState::running)
}

fn drive_busy(model: &Model) -> bool {
    matches!(model.write, WriteState::Running { .. }) || model.vault.is_busy()
}

pub(crate) fn saved(input: &Value) -> Option<Message> {
    Some(Message::Browse(BrowseMessage::DownloadEvent(
        Event::Finished(Outcome::Downloaded {
            path: input["path"].as_str()?.to_owned(),
            bytes: input["bytes"].as_u64()?,
        }),
    )))
}
