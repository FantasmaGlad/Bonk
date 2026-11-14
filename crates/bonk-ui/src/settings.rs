use bonk_hal::Lang;

use crate::model::{Effect, Effects, Focus, Model, WriteState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Main,
    Settings,
    Library,
}

/// `System` follows the machine at each start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LangChoice {
    #[default]
    System,
    En,
    Fr,
}

/// `System` follows the machine at each start; the theme never changes while the window runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl LangChoice {
    pub const ALL: [Self; 3] = [Self::System, Self::En, Self::Fr];

    /// As `bonk config` stores it under `lang`.
    #[must_use]
    pub fn config_value(self) -> &'static str {
        match self {
            Self::System => "auto",
            Self::En => "en",
            Self::Fr => "fr",
        }
    }

    #[must_use]
    pub fn from_lang(lang: Option<Lang>) -> Self {
        match lang {
            None => Self::System,
            Some(Lang::En) => Self::En,
            Some(Lang::Fr) => Self::Fr,
        }
    }

    /// Each language is named in itself: findable without reading the other.
    #[must_use]
    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::System => lang.pick("System language", "Langue du système"),
            Self::En => "English",
            Self::Fr => "Français",
        }
    }
}

impl ThemeChoice {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    /// As `bonk config` stores it under `theme`.
    #[must_use]
    pub fn config_value(self) -> &'static str {
        match self {
            Self::System => "auto",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    #[must_use]
    pub fn from_config(value: Option<&str>) -> Self {
        match value {
            Some("light") => Self::Light,
            Some("dark") => Self::Dark,
            _ => Self::System,
        }
    }

    #[must_use]
    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::System => lang.pick("System theme", "Thème du système"),
            Self::Light => lang.pick("Light", "Clair"),
            Self::Dark => lang.pick("Dark", "Sombre"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsMessage {
    Open,
    Close,
    Language(LangChoice),
    Theme(ThemeChoice),
    /// What the system answered for [`ThemeChoice::System`], once, at start.
    SystemDark(bool),
    Offline(bool),
}

impl Model {
    /// A relaunch would end the drive work running.
    #[must_use]
    pub fn can_relaunch(&self) -> bool {
        !matches!(self.write, WriteState::Running { .. }) && !self.vault.is_busy()
    }

    /// Why a language or theme choice is refused now, or what it does.
    #[must_use]
    pub fn relaunch_note(&self) -> &'static str {
        if self.can_relaunch() {
            self.lang.pick(
                "Bonk restarts at once to apply it.",
                "Bonk redémarre aussitôt pour l'appliquer.",
            )
        } else {
            self.lang.pick(
                "Unavailable while a drive is being written.",
                "Indisponible pendant l'écriture d'une clé.",
            )
        }
    }

    /// Dark or light as drawn now; `None` while the system has not answered.
    #[must_use]
    pub fn dark(&self) -> Option<bool> {
        match self.theme_choice {
            ThemeChoice::Light => Some(false),
            ThemeChoice::Dark => Some(true),
            ThemeChoice::System => self.system_dark,
        }
    }

    pub fn apply_settings(&mut self, message: SettingsMessage) -> Effects {
        match message {
            SettingsMessage::Open => {
                self.screen = Screen::Settings;
                self.focus = Some(Focus::CloseSettings);
                Vec::new()
            }
            SettingsMessage::Close => {
                self.screen = Screen::Main;
                self.focus = Some(Focus::Settings);
                Vec::new()
            }
            SettingsMessage::Language(choice) => {
                self.focus = Some(Focus::Language(choice));
                if choice == self.lang_choice || !self.can_relaunch() {
                    return Vec::new();
                }
                self.lang_choice = choice;
                vec![Effect::Relaunch {
                    key: "lang",
                    value: choice.config_value(),
                }]
            }
            SettingsMessage::Theme(choice) => {
                self.focus = Some(Focus::Theme(choice));
                if choice == self.theme_choice || !self.can_relaunch() {
                    return Vec::new();
                }
                self.theme_choice = choice;
                vec![Effect::Relaunch {
                    key: "theme",
                    value: choice.config_value(),
                }]
            }
            SettingsMessage::SystemDark(dark) => {
                // Read once: the theme does not follow the system while the window runs.
                self.system_dark.get_or_insert(dark);
                Vec::new()
            }
            SettingsMessage::Offline(on) => self.set_offline(on),
        }
    }

    fn set_offline(&mut self, on: bool) -> Effects {
        self.focus = Some(Focus::Offline);
        self.offline = on;
        let mut effects = vec![Effect::SaveSetting {
            key: "network.offline",
            value: if on { "true" } else { "false" },
        }];
        if on {
            self.catalog = crate::catalog::CatalogState::NotLoaded;
            self.menu_open = false;
            self.menu_filter = crate::catalog::MenuFilter::default();
            self.requested = None;
            self.advanced = true;
        } else {
            effects.extend(self.load_menu());
        }
        effects
    }

    #[must_use]
    pub fn settings_focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::CloseSettings];
        order.extend(LangChoice::ALL.map(Focus::Language));
        order.extend(ThemeChoice::ALL.map(Focus::Theme));
        order.extend([Focus::Internal, Focus::Virtual, Focus::All, Focus::Offline]);
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::ViewOptions;
    use crate::model::Message;

    fn model() -> Model {
        Model::new(ViewOptions::default(), Lang::En).0
    }

    #[test]
    fn changer_de_langue_enregistre_puis_relance() {
        let mut model = model();
        let _ = model.update(Message::Settings(SettingsMessage::Open));
        assert_eq!(model.screen, Screen::Settings);
        let effects = model.update(Message::Settings(SettingsMessage::Language(LangChoice::Fr)));
        assert_eq!(
            effects,
            [Effect::Relaunch {
                key: "lang",
                value: "fr"
            }]
        );
        assert!(
            model
                .update(Message::Settings(SettingsMessage::Language(LangChoice::Fr)))
                .is_empty()
        );
    }

    #[test]
    fn aucune_relance_pendant_une_ecriture() {
        let mut model = model();
        model.write = WriteState::Running {
            disk_id: "usb-a".to_owned(),
            pid: None,
            phase: bonk_core::write::Phase::Write,
            done: 0,
            total: None,
            rate: 0.0,
        };
        assert!(!model.can_relaunch());
        assert!(
            model
                .update(Message::Settings(SettingsMessage::Theme(ThemeChoice::Dark)))
                .is_empty()
        );
        assert_eq!(model.theme_choice, ThemeChoice::System);
    }

    #[test]
    fn le_theme_du_systeme_n_est_lu_qu_une_fois() {
        let mut model = model();
        assert_eq!(model.dark(), None);
        let _ = model.update(Message::Settings(SettingsMessage::SystemDark(true)));
        let _ = model.update(Message::Settings(SettingsMessage::SystemDark(false)));
        assert_eq!(model.dark(), Some(true));
        model.theme_choice = ThemeChoice::Light;
        assert_eq!(model.dark(), Some(false));
    }

    #[test]
    fn le_mode_hors_ligne_coupe_le_catalogue_et_refuse_les_adresses_web() {
        let draft = || WriteState::Draft {
            disk_id: "usb-a".to_owned(),
            image: String::new(),
            hash: String::new(),
            raw: false,
            mbr: true,
            opened: std::time::Instant::now(),
        };
        let mut model = model();
        model.write = draft();
        let effects = model.update(Message::Settings(SettingsMessage::Offline(true)));
        assert_eq!(
            effects,
            [Effect::SaveSetting {
                key: "network.offline",
                value: "true"
            }]
        );
        assert!(model.advanced, "l'image locale reste le seul chemin");
        assert!(model.request_image("debian-13".to_owned()).is_empty());
        assert!(model.requested.is_none());
        assert!(model.refuses_address(" HTTPS://exemple.org/a.iso"));
        assert!(!model.refuses_address("/home/u/a.iso"));
        let _ = model.update(Message::CatalogLoaded(Ok(crate::catalog::Shelf::default())));
        assert!(
            matches!(model.catalog, crate::catalog::CatalogState::NotLoaded),
            "une lecture lancée avant la bascule n'est pas reprise"
        );

        let effects = model.update(Message::Settings(SettingsMessage::Offline(false)));
        assert!(
            effects.contains(&Effect::LoadCatalog),
            "revenu en ligne, le menu se relit"
        );
        assert!(!model.refuses_address("https://exemple.org/a.iso"));
    }

    #[test]
    fn hors_ligne_des_le_demarrage() {
        let options = ViewOptions {
            offline: true,
            ..ViewOptions::default()
        };
        let mut model = Model::new(options, Lang::Fr).0;
        assert!(model.offline && model.advanced);
        model.write = WriteState::Draft {
            disk_id: "usb-a".to_owned(),
            image: String::new(),
            hash: String::new(),
            raw: false,
            mbr: true,
            opened: std::time::Instant::now(),
        };
        assert!(model.load_menu().is_empty());
    }

    #[test]
    fn valeurs_de_configuration() {
        for choice in ThemeChoice::ALL {
            assert_eq!(
                ThemeChoice::from_config(Some(choice.config_value())),
                choice
            );
        }
        assert_eq!(ThemeChoice::from_config(Some("bleu")), ThemeChoice::System);
        assert_eq!(LangChoice::from_lang(Some(Lang::Fr)).config_value(), "fr");
        assert_eq!(LangChoice::from_lang(None), LangChoice::System);
    }
}
