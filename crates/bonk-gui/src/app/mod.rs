mod catalog;
mod navigation;
mod settings;
#[cfg(test)]
mod tests;
mod view;

use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::time::Duration;

use bonk_hal::{Lang, enumerate_disks};
#[cfg(test)]
use iced::Size;
use iced::keyboard;
use iced::widget::scrollable;
use iced::{Subscription, Task, Theme};

#[cfg(test)]
use bonk_core::write::Phase;
use bonk_ui::library::{self, LibraryMessage};
pub(crate) use bonk_ui::model::{
    Effect, Focus, Message, Model, Nav, REFRESH_EVERY, WRITE_DELAY, WriteState,
};
pub(crate) use bonk_ui::vault::{VaultMessage, VaultState};

use crate::theme;
use crate::transfer;
#[cfg(test)]
use crate::transfer::Outcome;
use bonk_ui::command::ViewOptions;
use bonk_ui::settings::{LangChoice, SettingsMessage, ThemeChoice};

use navigation::{LIST, map_key};

/// What the window opens with: `bonk config`, the command line and the system.
#[derive(Debug, Default, Clone)]
pub(crate) struct Start {
    pub(crate) view: ViewOptions,
    /// `None`: the system's language.
    pub(crate) lang: Option<Lang>,
    pub(crate) theme: ThemeChoice,
    pub(crate) image: Option<String>,
}

#[derive(Debug)]
pub(crate) struct App {
    model: Model,
    /// `bonk` itself, except in tests.
    program: Option<PathBuf>,
}

impl Deref for App {
    type Target = Model;

    fn deref(&self) -> &Model {
        &self.model
    }
}

impl DerefMut for App {
    fn deref_mut(&mut self) -> &mut Model {
        &mut self.model
    }
}

fn snap(id: &'static str, y: f32) -> Task<Message> {
    iced::widget::operation::snap_to(id, scrollable::RelativeOffset { x: 0.0, y })
}

impl App {
    pub(crate) fn new(start: Start) -> (Self, Task<Message>) {
        let lang = start.lang.unwrap_or_else(crate::locale::detect);
        let (mut model, mut effects) = Model::new(start.view, lang);
        model.lang_choice = LangChoice::from_lang(start.lang);
        model.theme_choice = start.theme;
        if let Some(id) = start.image {
            effects.extend(model.request_image(id));
        }
        let app = Self {
            model,
            program: None,
        };
        app.draw_theme();
        let mut task = app.run(effects);
        if app.dark().is_none() {
            let system = iced::system::theme().map(|mode| {
                Message::Settings(SettingsMessage::SystemDark(mode == iced::theme::Mode::Dark))
            });
            task = Task::batch([system, task]);
        }
        (app, task)
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        let effects = self.model.update(message);
        #[cfg(target_os = "macos")]
        let effects = {
            let mut effects = effects;
            for id in crate::link_macos::take() {
                effects.extend(self.model.update(Message::RequestImage(id)));
            }
            effects
        };
        self.draw_theme();
        self.run(effects)
    }

    /// Once known, light or dark stays for the whole run.
    fn draw_theme(&self) {
        if let Some(dark) = self.dark() {
            theme::choose(dark);
        }
    }

    fn program(&self) -> PathBuf {
        self.program
            .clone()
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| PathBuf::from("bonk"))
    }

    fn run(&self, effects: Vec<Effect>) -> Task<Message> {
        Task::batch(effects.into_iter().map(|effect| self.accomplish(effect)))
    }

    fn accomplish(&self, effect: Effect) -> Task<Message> {
        match effect {
            Effect::Enumerate => Task::perform(
                async { enumerate_disks().map_err(|erreur| erreur.to_string()) },
                Message::Loaded,
            ),
            Effect::LoadCatalog => {
                let program = self.program();
                Task::perform(
                    async move { catalog::read(program) },
                    Message::CatalogLoaded,
                )
            }
            Effect::DetectTor => {
                Task::perform(async { bonk_catalog::tor::detect() }, Message::TorDetected)
            }
            Effect::Copy(text) => iced::clipboard::write(text),
            Effect::PickImage => Task::perform(pick_image(self.lang), Message::ImagePicked),
            Effect::SaveSetting { key, value } => {
                let program = self.program();
                Task::perform(async move { settings::save(&program, key, value) }, |_| ()).discard()
            }
            Effect::Relaunch { key, value } => {
                let program = self.program();
                Task::perform(
                    async move { settings::save(&program, key, value) && settings::relaunch() },
                    |started| started,
                )
                .then(|started| if started { iced::exit() } else { Task::none() })
            }
            Effect::StartWrite(args) => {
                Task::run(transfer::spawn(&self.program(), &args), Message::Write)
            }
            Effect::CancelWrite(pid) => {
                transfer::cancel(pid);
                Task::none()
            }
            Effect::ScrollList(y) => snap(LIST, y),
            Effect::ScrollMenu(y) => snap(catalog::MENU, y),
            Effect::RunVault(args) => Task::run(transfer::spawn(&self.program(), &args), |event| {
                Message::Vault(VaultMessage::Event(event))
            }),
            Effect::PickVaultIso => Task::perform(pick_iso(self.lang), |path| {
                Message::Vault(VaultMessage::Picked(path))
            }),
            Effect::ListKept => {
                let program = self.program();
                Task::perform(
                    async move {
                        catalog::run_json(&program, &["cache", "list", "--json"])
                            .and_then(|output| library::parse(&output))
                    },
                    |listed| Message::Library(LibraryMessage::Listed(listed)),
                )
            }
            Effect::RemoveKept(sha256) => {
                let program = self.program();
                Task::perform(
                    async move {
                        catalog::run_json(&program, &["cache", "clean", &sha256, "--json"])
                            .map(drop)
                    },
                    |removed| Message::Library(LibraryMessage::Removed(removed)),
                )
            }
        }
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![
            iced::time::every(REFRESH_EVERY).map(|_| Message::Tick),
            // A key taken by an input field (the image path) is not a command.
            iced::event::listen_with(|event, status, _window| match event {
                iced::Event::Keyboard(keyboard::Event::KeyPressed {
                    key,
                    modifiers,
                    repeat,
                    ..
                }) if status == iced::event::Status::Ignored => map_key(&key, modifiers, repeat),
                // On Wayland, winit 0.30 delivers no dropped file: the picker stays the safe way.
                iced::Event::Window(iced::window::Event::FileDropped(path)) => {
                    Some(Message::ImagePicked(Some(path)))
                }
                _ => None,
            }),
        ];
        // The countdown before Write is enabled refreshes while the panel is open.
        if matches!(self.write, WriteState::Draft { .. }) {
            subscriptions
                .push(iced::time::every(Duration::from_millis(500)).map(|_| Message::Clock));
        }
        Subscription::batch(subscriptions)
    }

    pub(crate) fn theme(&self) -> Theme {
        theme::bonk()
    }
}

async fn pick_iso(lang: Lang) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title(lang.pick(
            "Choose the ISO to add to the vault",
            "Choisir l'ISO à ajouter au coffre",
        ))
        .add_filter(lang.pick("ISO images", "Images ISO"), &["iso", "ISO"])
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

async fn pick_image(lang: Lang) -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title(lang.pick("Choose the image to write", "Choisir l'image à écrire"))
        .add_filter(
            lang.pick("Disk images", "Images de disque"),
            &["iso", "img", "raw", "gz", "xz", "zst"],
        )
        .add_filter(lang.pick("All files", "Tous les fichiers"), &["*"])
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}
