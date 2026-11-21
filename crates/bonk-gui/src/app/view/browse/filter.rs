use iced::Element;
use iced::widget::{pick_list, row};

use bonk_hal::Lang;
use bonk_ui::browse::{BrowseMessage, desktop_label, media_label, options};
use bonk_ui::catalog::System;

use super::super::super::{App, Message};

type Entries = Vec<(String, String)>;

impl App {
    pub(super) fn catalog_filters<'a>(&'a self, systems: &'a [System]) -> Element<'a, Message> {
        let lang = self.lang;
        let offered = options(systems, &self.terms);
        let filter = &self.browse.filter;
        let (uses, bases) = (self.terms.uses(), self.terms.bases());
        let label_all = |ids: &[String], label: &dyn Fn(&str) -> String| -> Entries {
            ids.iter().map(|id| (id.clone(), label(id))).collect()
        };
        row![
            list(
                lang.pick("All uses", "Tous les usages"),
                label_all(&offered.uses, &|id| uses.label(id, lang)),
                filter.uses.as_ref(),
                BrowseMessage::Uses,
            ),
            list(
                lang.pick("All bases", "Toutes les bases"),
                label_all(&offered.bases, &|id| bases.label(id, lang)),
                filter.base.as_ref(),
                BrowseMessage::Base,
            ),
            list(
                lang.pick("All desktops", "Tous les bureaux"),
                label_all(&offered.desktops, &|id| desktop_label(id, lang)),
                filter.desktop.as_ref(),
                BrowseMessage::Desktop,
            ),
            list(
                media_all(lang),
                label_all(&offered.media, &|id| media_label(id, lang)),
                filter.media.as_ref(),
                BrowseMessage::Media,
            ),
        ]
        .spacing(8)
        .into()
    }
}

fn media_all(lang: Lang) -> &'static str {
    lang.pick("All media", "Tous les supports")
}

fn list<'a>(
    all: &'static str,
    entries: Entries,
    chosen: Option<&String>,
    message: fn(Option<String>) -> BrowseMessage,
) -> Element<'a, Message> {
    let shown = chosen
        .and_then(|id| entries.iter().find(|(known, _)| known == id))
        .map_or_else(|| all.to_owned(), |(_, label)| label.clone());
    let mut labels: Vec<String> = vec![all.to_owned()];
    labels.extend(entries.iter().map(|(_, label)| label.clone()));
    pick_list(labels, Some(shown), move |picked: String| {
        let id = entries
            .iter()
            .find(|(_, label)| *label == picked)
            .map(|(id, _)| id.clone());
        Message::Browse(message(id))
    })
    .text_size(13)
    .into()
}
