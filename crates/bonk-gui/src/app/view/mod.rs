use iced::widget::{center, column, container, opaque, stack, text};
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
        let entries = self.entries();
        let shown = entries.len();
        let body = match self.active_panel() {
            Some(panel) => column![panel, self.disk_list(entries)].spacing(8).into(),
            None => self.disk_list(entries),
        };
        let mut content = column![self.header(shown)].spacing(14);
        if let Some(notice) = self.requested_notice() {
            content = content.push(notice);
        }
        content.push(body).push(self.command_bar()).into()
    }

    pub(super) fn has_focus(&self, target: Focus) -> bool {
        self.focus.as_ref() == Some(&target)
    }
}
