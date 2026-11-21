mod command;
mod store;
#[cfg(test)]
mod tests;

pub(crate) use command::*;
pub(crate) use store::*;

pub(crate) enum Kind {
    Bool,
    Choice(&'static [&'static str]),
}

pub(crate) struct Key {
    pub(crate) name: &'static str,
    pub(crate) kind: Kind,
    pub(crate) default: &'static str,
    pub(crate) help: &'static str,
}

pub(crate) const KEYS: &[Key] = &[
    Key {
        name: "lang",
        kind: Kind::Choice(&["auto", "en", "fr"]),
        default: "auto",
        help: "window language; auto follows the system; the command line is always in English",
    },
    Key {
        name: "theme",
        kind: Kind::Choice(&["auto", "light", "dark"]),
        default: "auto",
        help: "window theme; auto follows the system at each start",
    },
    Key {
        name: "log.format",
        kind: Kind::Choice(&["text", "json"]),
        default: "text",
        help: "log format when --log-format is not given",
    },
    Key {
        name: "view.show_internal",
        kind: Kind::Bool,
        default: "false",
        help: "also offer internal non-system disks (window, list, info)",
    },
    Key {
        name: "view.show_all",
        kind: Kind::Bool,
        default: "false",
        help: "also show excluded disks (window, list)",
    },
    Key {
        name: "view.allow_virtual",
        kind: Kind::Bool,
        default: "false",
        help: "test mode: allow virtual disks (window, list, info)",
    },
    Key {
        name: "network.offline",
        kind: Kind::Bool,
        default: "false",
        help: "window: never read the catalog nor download; local image files only",
    },
    Key {
        name: "network.incognito",
        kind: Kind::Bool,
        default: "false",
        help: "window: start in incognito; nothing reaches the network until a local Tor answers",
    },
    Key {
        name: "notifications",
        kind: Kind::Bool,
        default: "true",
        help: "window: each step of a write shown in the system's notifications",
    },
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewDefaults {
    pub show_internal: bool,
    pub show_all: bool,
    pub allow_virtual: bool,
    pub offline: bool,
    /// The window starts in incognito and waits for Tor before any network use.
    pub incognito: bool,
    pub notify: bool,
    /// `None`: the system's.
    pub lang: Option<bonk_hal::Lang>,
    /// `auto`, `light` or `dark`.
    pub theme: &'static str,
}

impl ViewDefaults {
    /// To list or verify, the settings widen what was asked; writing uses what was asked alone.
    #[must_use]
    pub fn widen(&self, asked: bonk_hal::FilterOptions) -> bonk_hal::FilterOptions {
        bonk_hal::FilterOptions {
            show_internal: asked.show_internal || self.show_internal,
            allow_virtual: asked.allow_virtual || self.allow_virtual,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowRequest {
    pub view: ViewDefaults,
    pub image: Option<String>,
}

pub(crate) fn variable_name(key: &str) -> String {
    format!("BONK_{}", key.replace('.', "_").to_uppercase())
}

pub(crate) fn find_key(name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.name == name)
}

pub(crate) fn normalize(key: &Key, value: &str) -> Result<String, String> {
    let value = value.trim();
    match &key.kind {
        Kind::Bool => match value.to_lowercase().as_str() {
            "true" | "1" | "oui" | "yes" | "on" => Ok("true".to_owned()),
            "false" | "0" | "non" | "no" | "off" => Ok("false".to_owned()),
            _ => Err(format!(
                "'{value}' is not a boolean for {} (true or false)",
                key.name
            )),
        },
        Kind::Choice(choices) => {
            let lower = value.to_lowercase();
            if choices.contains(&lower.as_str()) {
                Ok(lower)
            } else {
                Err(format!(
                    "'{value}' is not a value of {} (values: {})",
                    key.name,
                    choices.join(", ")
                ))
            }
        }
    }
}
