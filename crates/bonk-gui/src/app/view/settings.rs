use iced::widget::{Column, button, checkbox, column, row, scrollable, text, toggler};
use iced::{Element, Fill};

use bonk_ui::settings::{LangChoice, SettingsMessage, ThemeChoice};

use crate::theme;

use super::super::{App, Focus, Message};
use super::ring;

/// One choice of a group: the chosen one is outlined, the others look like plain cards.
fn option<'a>(
    label: &'a str,
    chosen: bool,
    message: Option<Message>,
    focused: bool,
) -> Element<'a, Message> {
    ring(
        button(text(label).size(14))
            .on_press_maybe(message)
            .padding([8, 16])
            .style(theme::card_button(chosen)),
        focused,
    )
}

fn section<'a>(title: &'a str, note: &'a str, body: Element<'a, Message>) -> Element<'a, Message> {
    column![
        text(title).size(18),
        text(note).size(13).color(theme::muted()),
        body
    ]
    .spacing(8)
    .into()
}

impl App {
    pub(super) fn settings_page(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let title = row![
            text(lang.pick("Settings", "Paramètres"))
                .size(28)
                .width(Fill),
            ring(
                button(text(lang.pick("Back", "Retour")).size(14))
                    .on_press(Message::Settings(SettingsMessage::Close))
                    .padding([6, 14])
                    .style(theme::card_button(false)),
                self.has_focus(Focus::CloseSettings),
            ),
        ]
        .align_y(iced::Center);
        let sections = column![
            self.language_section(),
            self.theme_section(),
            self.disks_section(),
            self.network_section()
        ]
        .spacing(28);
        column![title, scrollable(sections).height(Fill)]
            .spacing(20)
            .into()
    }

    /// Why a choice is refused now, or what it does.
    fn relaunch_note(&self) -> &'static str {
        let lang = self.lang;
        if self.can_relaunch() {
            lang.pick(
                "Bonk restarts at once to apply it.",
                "Bonk redémarre aussitôt pour l'appliquer.",
            )
        } else {
            lang.pick(
                "Unavailable while a drive is being written.",
                "Indisponible pendant l'écriture d'une clé.",
            )
        }
    }

    fn language_section(&self) -> Element<'_, Message> {
        let choices = LangChoice::ALL.map(|choice| {
            option(
                choice.label(self.lang),
                choice == self.lang_choice,
                self.can_relaunch()
                    .then_some(Message::Settings(SettingsMessage::Language(choice))),
                self.has_focus(Focus::Language(choice)),
            )
        });
        let title = self.lang.pick("Language", "Langue");
        section(title, self.relaunch_note(), row(choices).spacing(8).into())
    }

    fn theme_section(&self) -> Element<'_, Message> {
        let choices = ThemeChoice::ALL.map(|choice| {
            option(
                choice.label(self.lang),
                choice == self.theme_choice,
                self.can_relaunch()
                    .then_some(Message::Settings(SettingsMessage::Theme(choice))),
                self.has_focus(Focus::Theme(choice)),
            )
        });
        let title = self.lang.pick("Theme", "Thème");
        section(title, self.relaunch_note(), row(choices).spacing(8).into())
    }

    /// The safety filter's expert options: off by default, kept for the next start.
    fn disks_section(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let boxes = [
            (
                self.show_internal,
                lang.pick("Internal non-system disks", "Disques internes non système"),
                Message::ToggleInternal as fn(bool) -> Message,
                Focus::Internal,
            ),
            (
                self.allow_virtual,
                lang.pick("Virtual disks (test mode)", "Disques virtuels (mode test)"),
                Message::ToggleVirtual,
                Focus::Virtual,
            ),
            (
                self.show_all,
                lang.pick(
                    "Also show excluded disks",
                    "Afficher aussi les disques écartés",
                ),
                Message::ToggleAll,
                Focus::All,
            ),
        ];
        let rows = boxes.map(|(on, label, message, focus)| {
            ring(
                checkbox(on)
                    .label(label)
                    .on_toggle(message)
                    .size(18)
                    .style(theme::checkbox_style),
                self.has_focus(focus),
            )
        });
        section(
            lang.pick("Disks", "Disques"),
            lang.pick(
                "For experts: the safety filter still refuses system disks.",
                "Pour les experts : le filtre de sécurité refuse toujours les disques système.",
            ),
            Column::with_children(rows).spacing(4).into(),
        )
    }

    /// For whoever wants a tool that never reaches the network, so that nothing reveals their address.
    fn network_section(&self) -> Element<'_, Message> {
        let lang = self.lang;
        let switch = toggler(self.offline)
            .label(lang.pick("Offline mode", "Mode hors ligne"))
            .on_toggle(|on| Message::Settings(SettingsMessage::Offline(on)))
            .size(20);
        section(
            lang.pick("Network", "Réseau"),
            lang.pick(
                "Never connect to the Internet: the catalog is hidden and only image files on this computer can be written.",
                "Ne jamais se connecter à Internet : le catalogue est masqué, seules les images présentes sur cet ordinateur s'écrivent.",
            ),
            ring(switch, self.has_focus(Focus::Offline)),
        )
    }
}
