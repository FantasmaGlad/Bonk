use bonk_hal::Lang;
use bonk_ui::catalog::{
    CatalogState, ChoiceSource, System, all_arches, all_bases, all_categories, all_systems,
    arch_label, official_page_note,
};
use bonk_ui::model::{Model, WriteState};
use serde_json::{Value, json};

pub(in crate::screen) fn menu_state(model: &Model) -> Value {
    let lang = model.lang;
    let systems = match (&model.catalog, model.offline) {
        (CatalogState::Ready(systems), false) => systems,
        _ => return json!({"ready": false, "line": waiting_line(model)}),
    };
    let lists = model.menu_filter.lists(systems, &model.terms);
    let (uses, bases) = (model.terms.uses(), model.terms.bases());
    let categories: Vec<String> = lists
        .categories
        .iter()
        .map(|id| uses.label(id, lang))
        .collect();
    let base_names: Vec<String> = lists.bases.iter().map(|id| bases.label(id, lang)).collect();
    json!({
        "ready": true,
        "open": model.menu_open,
        "button": chosen_title(model).unwrap_or_else(|| lang.pick("Choose a system...", "Choisir un système...").to_owned()),
        "all_categories": all_categories(lang),
        "all_bases": all_bases(lang),
        "all_systems": all_systems(lang),
        "category": model.menu_filter.category.as_deref().map(|id| uses.label(id, lang)),
        "base": model.menu_filter.base.as_deref().map(|id| bases.label(id, lang)),
        "system": model.menu_filter.system,
        "categories": categories,
        "bases": base_names,
        "systems": lists.systems,
        "search": model.menu_filter.search,
        "search_hint": lang.pick("Search by name", "Rechercher par nom"),
        "all_arches": all_arches(lang),
        "arch": model.menu_filter.arch.as_deref().map(|id| arch_label(id, lang)),
        "arches": lists.arches.iter().map(|id| arch_label(id, lang)).collect::<Vec<_>>(),
        "rows": rows(model, systems),
    })
}

fn waiting_line(model: &Model) -> String {
    let lang = model.lang;
    if model.offline {
        return lang
            .pick(
                "Offline mode: choose an image file on this device.",
                "Mode hors ligne : choisissez une image présente sur cet appareil.",
            )
            .to_owned();
    }
    match &model.catalog {
        CatalogState::Failed(error) => error.clone(),
        _ => lang
            .pick(
                "Reading the list of systems...",
                "Lecture de la liste des systèmes...",
            )
            .to_owned(),
    }
}

/// Only writable rows carry an index: it is the position the app sends back when one is touched.
fn rows(model: &Model, systems: &[System]) -> Vec<Value> {
    // Long list: building and rereading it at every tick would slow the whole screen for nothing.
    if !model.menu_open {
        return Vec::new();
    }
    let mut index = 0;
    let mut rows = Vec::new();
    for system in model.menu_filter.apply(systems) {
        for choice in model.menu_filter.choices(system) {
            let usable = matches!(choice.source, ChoiceSource::Download { .. });
            rows.push(row(model.lang, choice, usable.then_some(index)));
            index += usize::from(usable);
        }
    }
    rows
}

fn row(lang: Lang, choice: &bonk_ui::catalog::Choice, index: Option<usize>) -> Value {
    let note = match &choice.source {
        ChoiceSource::OfficialPage(page) => Some(official_page_note(page, lang)),
        _ => choice.note.clone(),
    };
    json!({
        "title": format!("{} {}", choice.name, choice.title(lang)),
        "note": note,
        "index": index,
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
