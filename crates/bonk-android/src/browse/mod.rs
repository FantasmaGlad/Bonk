pub(crate) mod download;

use bonk_hal::Lang;
use bonk_ui::browse::{
    BrowseMessage, Options, desktop_label, media_label, options, variant_facts, writable_count,
};
use bonk_ui::catalog::{
    CatalogState, Choice, ChoiceSource, System, affiliation_note, official_page_note, vault_note,
};
use bonk_ui::model::{Message, Model};
use bonk_ui::settings::Screen;
use serde_json::{Value, json};

/// The header button; `null` offline, where the catalog is never read.
pub(crate) fn button(model: &Model) -> Value {
    if model.offline {
        return Value::Null;
    }
    json!(model.lang.pick("Catalog", "Catalogue"))
}

pub(crate) fn state(model: &Model) -> Value {
    if model.screen != Screen::Catalog {
        return Value::Null;
    }
    let lang = model.lang;
    let mut view = json!({
        "title": lang.pick("Catalog", "Catalogue"),
        "back": lang.pick("Back", "Retour"),
        "notice": model.catalog_notice(),
    });
    match (&model.catalog, model.opened_system()) {
        (CatalogState::Ready(_), Some(system)) => view["system"] = system_page(model, system),
        (CatalogState::Ready(systems), None) => view["grid"] = grid(model, systems),
        (CatalogState::Failed(error), _) => view["waiting"] = json!(error),
        _ => {
            view["waiting"] = json!(lang.pick(
                "Reading the list of systems...",
                "Lecture de la liste des systèmes...",
            ));
        }
    }
    view
}

fn grid(model: &Model, systems: &[System]) -> Value {
    let lang = model.lang;
    let cards: Vec<Value> = model
        .browsed()
        .iter()
        .map(|system| {
            json!({
                "name": system.name,
                "count": writable_count(system, lang),
                "icon": system.icon,
            })
        })
        .collect();
    json!({
        "search": model.browse.filter.search,
        "search_hint": lang.pick("Search: Ubuntu, Fedora, Windows…", "Rechercher : Ubuntu, Fedora, Windows…"),
        "filters": filters(model, &options(systems, &model.terms)),
        "cards": cards,
        "none": lang.pick("No system matches.", "Aucun système ne correspond."),
    })
}

fn filters(model: &Model, offered: &Options) -> Vec<Value> {
    let lang = model.lang;
    let filter = &model.browse.filter;
    let (uses, bases) = (model.terms.uses(), model.terms.bases());
    let list = |kind: &str,
                all: &str,
                ids: &[String],
                chosen: Option<&String>,
                label: &dyn Fn(&str) -> String| {
        json!({
            "kind": kind,
            "all": all,
            "options": ids.iter().map(|id| label(id)).collect::<Vec<_>>(),
            "chosen": chosen.map(|id| label(id)),
        })
    };
    vec![
        list(
            "uses",
            lang.pick("All uses", "Tous les usages"),
            &offered.uses,
            filter.uses.as_ref(),
            &|id| uses.label(id, lang),
        ),
        list(
            "base",
            lang.pick("All bases", "Toutes les bases"),
            &offered.bases,
            filter.base.as_ref(),
            &|id| bases.label(id, lang),
        ),
        list(
            "desktop",
            lang.pick("All desktops", "Tous les bureaux"),
            &offered.desktops,
            filter.desktop.as_ref(),
            &|id| desktop_label(id, lang),
        ),
        list(
            "media",
            lang.pick("All media", "Tous les supports"),
            &offered.media,
            filter.media.as_ref(),
            &|id| media_label(id, lang),
        ),
    ]
}

fn system_page(model: &Model, system: &System) -> Value {
    let lang = model.lang;
    let variants: Vec<Value> = system
        .choices
        .iter()
        .filter(|choice| model.browse.filter.keeps_choice(system, choice))
        .map(|choice| variant(choice, lang))
        .collect();
    json!({
        "name": system.name,
        "icon": system.icon,
        "affiliation": system.not_affiliated_with.as_deref().map(|owner| affiliation_note(owner, lang)),
        "back": lang.pick("All systems", "Tous les systèmes"),
        "write": lang.pick("Write this image", "Écrire cette image"),
        "download": lang.pick("Download", "Télécharger"),
        "busy": download::running(model),
        "downloading": download::panel(model, system),
        "variants": variants,
    })
}

/// `id` only when Bonk can download and write it; otherwise `note` says why not.
fn variant(choice: &Choice, lang: Lang) -> Value {
    let (id, note) = match &choice.source {
        ChoiceSource::Download { .. } => (Some(choice.id.as_str()), None),
        ChoiceSource::OfficialPage(page) => (None, Some(official_page_note(page, lang))),
        ChoiceSource::Vault { .. } => (None, Some(vault_note(lang).to_owned())),
        ChoiceSource::NotForUsb => (
            None,
            Some(
                lang.pick(
                    "Does not start from a drive",
                    "Ne démarre pas depuis une clé",
                )
                .to_owned(),
            ),
        ),
    };
    json!({
        "title": format!("{} {}", choice.name, choice.title(lang)),
        "facts": variant_facts(choice, lang),
        "id": id,
        "note": note,
    })
}

pub(crate) fn message_of(input: &Value, model: &Model) -> Option<Message> {
    let text = |key: &str| input[key].as_str().map(str::to_owned);
    let browse = match input["type"].as_str()? {
        "catalog" => match input["action"].as_str()? {
            "open" => BrowseMessage::Open,
            "close" => BrowseMessage::Close,
            "close_system" => BrowseMessage::CloseSystem,
            _ => return None,
        },
        "catalog_search" => BrowseMessage::Search(text("value").unwrap_or_default()),
        "catalog_system" => BrowseMessage::OpenSystem(text("name")?),
        "catalog_write" => BrowseMessage::Write(text("id")?),
        "catalog_download" => download::message_of(input, model)?,
        "catalog_filter" => filter_message(model, input["kind"].as_str()?, text("value"))?,
        _ => return None,
    };
    Some(Message::Browse(browse))
}

/// The app sends back the label it showed; no label removes the filter.
fn filter_message(model: &Model, kind: &str, label: Option<String>) -> Option<BrowseMessage> {
    let lang = model.lang;
    let CatalogState::Ready(systems) = &model.catalog else {
        return None;
    };
    let offered = options(systems, &model.terms);
    let find = |ids: &[String], name: &dyn Fn(&str) -> String| {
        label
            .as_ref()
            .and_then(|label| ids.iter().find(|id| &name(id) == label).cloned())
    };
    Some(match kind {
        "uses" => BrowseMessage::Uses(find(&offered.uses, &|id| {
            model.terms.uses().label(id, lang)
        })),
        "base" => BrowseMessage::Base(find(&offered.bases, &|id| {
            model.terms.bases().label(id, lang)
        })),
        "desktop" => BrowseMessage::Desktop(find(&offered.desktops, &|id| desktop_label(id, lang))),
        "media" => BrowseMessage::Media(find(&offered.media, &|id| media_label(id, lang))),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use bonk_ui::catalog::Choice;

    use super::*;

    fn with_debian() -> Model {
        let choice = Choice {
            id: "debian-live".to_owned(),
            name: "Debian".to_owned(),
            edition: None,
            label: "13".to_owned(),
            end_of_life: false,
            note: None,
            source: ChoiceSource::NotForUsb,
            desktop: Some("kde".to_owned()),
            media: "usb".to_owned(),
            arch: "x86_64".to_owned(),
            boot: None,
        };
        Model {
            lang: Lang::Fr,
            catalog: CatalogState::Ready(vec![System {
                id: String::new(),
                name: "Debian".to_owned(),
                uses: Vec::new(),
                base: None,
                icon: None,
                choices: vec![choice],
                not_affiliated_with: None,
            }]),
            ..Model::default()
        }
    }

    #[test]
    fn un_libelle_de_filtre_revient_a_son_identifiant() {
        let model = with_debian();
        let picked = json!({"type": "catalog_filter", "kind": "desktop", "value": "KDE Plasma"});
        assert!(matches!(
            message_of(&picked, &model),
            Some(Message::Browse(BrowseMessage::Desktop(Some(id)))) if id == "kde"
        ));
        let removed = json!({"type": "catalog_filter", "kind": "desktop"});
        assert!(matches!(
            message_of(&removed, &model),
            Some(Message::Browse(BrowseMessage::Desktop(None)))
        ));
    }

    #[test]
    fn l_onglet_n_apparait_qu_en_ligne_et_sur_son_ecran() {
        let mut model = with_debian();
        assert_eq!(state(&model), Value::Null);
        assert_eq!(button(&model), json!("Catalogue"));
        model.screen = Screen::Catalog;
        assert_eq!(state(&model)["grid"]["cards"][0]["name"], "Debian");
        model.offline = true;
        assert_eq!(button(&model), Value::Null);
    }
}
