use iced::widget::{
    Column, center, column, container, opaque, responsive, row, scrollable, stack, text,
};
use iced::{Element, Fill};

use crate::theme;

use bonk_ui::settings::Screen;

use super::{App, Focus, Message};

mod command_bar;
mod disk;
mod header;
mod library;
mod list;
mod panel;
mod settings;
mod vault;

#[cfg(test)]
pub(super) use bonk_ui::text::secondes_restantes;

/// From this width the disks and the write panel sit side by side.
pub(super) const WIDE: f32 = 900.0;
const DISKS_WIDTH: f32 = 360.0;

pub(super) use disk::internal_dialog;

/// Visible only when the widget has focus.
pub(super) fn ring<'a>(
    content: impl Into<Element<'a, Message>>,
    focused: bool,
) -> Element<'a, Message> {
    container(content)
        .padding(6)
        .style(theme::focus_ring(focused))
        .into()
}

pub(super) fn boxed<'a>(body: Element<'a, Message>) -> Element<'a, Message> {
    container(body)
        .padding(12)
        .width(Fill)
        .style(theme::panel)
        .into()
}

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
        // Until the system says light or dark, nothing is drawn: no flash of the wrong theme.
        if self.dark().is_none() {
            return container(text("")).width(Fill).height(Fill).into();
        }
        let page = match self.screen {
            Screen::Main => self.main_page(),
            Screen::Settings => self.settings_page(),
            Screen::Library => self.library_page(),
        };
        let window = container(page).padding(20).width(Fill).height(Fill);
        match self.internal_to_confirm() {
            Some(disk) => stack![window, opaque(center(internal_dialog(disk, self.lang)))].into(),
            None => window.into(),
        }
    }

    fn main_page(&self) -> Element<'_, Message> {
        let mut content = column![self.header(self.entries().len())].spacing(14);
        if let Some(notice) = self.requested_notice() {
            content = content.push(notice);
        }
        let body = responsive(move |size| {
            if size.width >= WIDE {
                self.wide_body()
            } else {
                self.narrow_body()
            }
        });
        content.push(body).push(self.command_bar()).into()
    }

    /// One column: each panel under its disk.
    fn narrow_body(&self) -> Element<'_, Message> {
        let list = self.disk_list(self.entries(), true);
        match self.active_panel() {
            Some(panel) => column![panel, list].spacing(8).into(),
            None => list,
        }
    }

    fn wide_body(&self) -> Element<'_, Message> {
        let side = self.side_panels().unwrap_or_else(|| {
            container(text(self.side_hint()).size(14).color(theme::muted()))
                .padding(12)
                .into()
        });
        row![
            container(self.disk_list(self.entries(), false)).width(DISKS_WIDTH),
            scrollable(container(side).padding([0, 4]))
                .height(Fill)
                .width(Fill),
        ]
        .spacing(16)
        .into()
    }

    /// A running write first: its disk may be gone from the list. Then the chosen disk's write panel and
    /// the vault being prepared.
    fn side_panels(&self) -> Option<Element<'_, Message>> {
        if let Some(panel) = self.active_panel() {
            return Some(panel);
        }
        let entries = self.entries();
        let find = |id: &str| {
            entries
                .iter()
                .find(|(disk, _)| disk.id == id)
                .map(|(disk, _)| *disk)
        };
        let write = self.selected.as_deref().and_then(find);
        let vault = self.vault.disk_id().and_then(find);
        let panels: Vec<_> = [
            write.and_then(|disk| self.write_panel(disk)),
            vault.and_then(|disk| self.vault_panel(disk)),
        ]
        .into_iter()
        .flatten()
        .collect();
        (!panels.is_empty()).then(|| Column::with_children(panels).spacing(8).into())
    }

    fn side_hint(&self) -> &'static str {
        self.lang.pick(
            "Choose a disk to write an image to it.",
            "Choisissez un disque pour y écrire une image.",
        )
    }

    pub(super) fn has_focus(&self, target: Focus) -> bool {
        self.focus.as_ref() == Some(&target)
    }
}
