use iced::widget::{button, image, row, text, tooltip};
use iced::{Element, Length};

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

const SIDE: f32 = 36.0;
const GLYPH: f32 = 20.0;

impl App {
    pub(super) fn incognito_control(&self) -> Option<Element<'_, Message>> {
        if self.offline {
            return None;
        }
        let control = tooltip(
            ring(self.incognito_button(), self.has_focus(Focus::Tor)),
            text(self.incognito_label()).size(13),
            tooltip::Position::Bottom,
        )
        .style(theme::panel);
        Some(match self.tor {
            Some(_) => control.into(),
            None => row![control, self.tor_project_link()]
                .align_y(iced::Center)
                .into(),
        })
    }

    fn incognito_button(&self) -> Element<'_, Message> {
        let c = theme::colors();
        let glyph = match (&self.tor, self.use_tor) {
            (_, true) => c.on_accent,
            (None, false) => c.muted,
            (Some(_), false) => c.ink,
        };
        button(
            image(theme::incognito_icon(glyph))
                .width(GLYPH)
                .height(GLYPH),
        )
        .on_press_maybe(
            (self.tor.is_some() || self.use_tor).then_some(Message::ToggleTor(!self.use_tor)),
        )
        .width(Length::Fixed(SIDE))
        .height(Length::Fixed(SIDE))
        .padding((SIDE - GLYPH) / 2.0)
        .style(theme::round_button(self.use_tor))
        .into()
    }

    fn tor_project_link(&self) -> Element<'_, Message> {
        button(text("torproject.org").size(13))
            .on_press(Message::OpenTorProject)
            .padding([2, 4])
            .style(theme::link_button)
            .into()
    }
}
