use iced::widget::{Column, button, column, row, scrollable, text};
use iced::{Element, Fill};

use bonk_hal::format_size;
use bonk_ui::library::{Kept, LibraryMessage, Listing, day};

use crate::theme;

use super::super::{App, Focus, Message};
use super::{boxed, ring};

impl App {
    pub(super) fn library_page(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let title = row![
            text(lang.pick("Library", "Bibliothèque"))
                .size(28)
                .width(Fill),
            ring(
                button(text(lang.pick("Back", "Retour")).size(14))
                    .on_press(Message::Library(LibraryMessage::Close))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::CloseLibrary),
            ),
        ]
        .align_y(iced::Center);
        let note = text(lang.pick(
            "Images already downloaded. Writing one does not download it again; its hash is checked before.",
            "Images déjà téléchargées. En écrire une ne la télécharge pas de nouveau ; son empreinte est contrôlée avant.",
        ))
        .size(13)
        .color(theme::muted());
        let mut content = column![title, note].spacing(14);
        if let Some(error) = &self.library.error {
            content = content.push(text(error.as_str()).size(13));
        }
        content.push(self.kept_list()).into()
    }

    fn kept_list(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let quiet = |line: String| text(line).size(14).color(theme::muted()).into();
        match &self.library.listing {
            Listing::Loading => quiet(lang.pick("Reading…", "Lecture…").to_owned()),
            Listing::Failed(error) => quiet(format!(
                "{} {error}",
                lang.pick("Cannot read the cache:", "Cache illisible :")
            )),
            Listing::Ready(kept) if kept.is_empty() => quiet(
                lang.pick(
                    "No image kept yet: each image written from the catalog stays here.",
                    "Aucune image gardée : chaque image écrite depuis le catalogue reste ici.",
                )
                .to_owned(),
            ),
            Listing::Ready(kept) => scrollable(
                Column::with_children(kept.iter().map(|one| self.kept_row(one)))
                    .spacing(8)
                    .padding([0, 4]),
            )
            .height(Fill)
            .into(),
        }
    }

    fn kept_row<'a>(&'a self, kept: &'a Kept) -> Element<'a, Message> {
        let lang = self.lang;
        let title = self.kept_label(&kept.sha256).unwrap_or(&kept.name);
        let mut facts = vec![format_size(kept.bytes, lang)];
        if let Some(seconds) = kept.modified {
            facts.push(day(seconds));
        }
        if kept.partial {
            facts.push(
                lang.pick("incomplete download", "téléchargement incomplet")
                    .to_owned(),
            );
        }
        let short = kept.sha256.get(..16).unwrap_or(&kept.sha256);
        let details = column![
            text(title).size(16),
            text(facts.join(" · ")).size(13).color(theme::muted()),
            text(format!("SHA-256 {short}…"))
                .size(12)
                .font(theme::MONO)
                .color(theme::muted()),
        ]
        .spacing(2)
        .width(Fill);
        boxed(
            row![details, self.kept_actions(kept)]
                .spacing(12)
                .align_y(iced::Center)
                .into(),
        )
    }

    /// An incomplete download is never written: it only resumes with the next write of its image.
    fn kept_actions<'a>(&'a self, kept: &'a Kept) -> Element<'a, Message> {
        let lang = self.lang;
        let mut actions = row![].spacing(8).align_y(iced::Center);
        if !kept.partial {
            actions = actions.push(ring(
                button(text(lang.pick("Write", "Écrire")).size(14))
                    .on_press(Message::Library(LibraryMessage::Write(kept.sha256.clone())))
                    .padding([6, 14])
                    .style(theme::primary_button),
                self.has_focus(Focus::KeptWrite(kept.sha256.clone())),
            ));
        }
        let remove = if self.library.removing.as_deref() == Some(kept.sha256.as_str()) {
            lang.pick("Removing…", "Suppression…")
        } else {
            lang.pick("Remove", "Supprimer")
        };
        let message = Message::Library(LibraryMessage::Remove(kept.sha256.clone()));
        actions
            .push(ring(
                button(text(remove).size(14))
                    .on_press_maybe(self.can_remove_kept().then_some(message))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::KeptRemove(kept.sha256.clone())),
            ))
            .into()
    }
}
