use bonk_hal::Lang;

use crate::catalog::{Choice, ChoiceSource, System};

use super::{desktop_label, media_label};

/// Under a system card: how many of its images Bonk can download and write.
#[must_use]
pub fn writable_count(system: &System, lang: Lang) -> String {
    let count = system
        .choices
        .iter()
        .filter(|choice| matches!(choice.source, ChoiceSource::Download { .. }))
        .count();
    match (count, lang) {
        (1, _) => lang.pick("1 image to write", "1 image à écrire").to_owned(),
        (n, Lang::En) => format!("{n} images to write"),
        (n, Lang::Fr) => format!("{n} images à écrire"),
    }
}

/// Desktop, medium and the catalog note, on one line.
#[must_use]
pub fn variant_facts(choice: &Choice, lang: Lang) -> String {
    [
        choice.desktop.as_deref().map(|id| desktop_label(id, lang)),
        choice.media.as_deref().map(|id| media_label(id, lang)),
        choice.note.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}
