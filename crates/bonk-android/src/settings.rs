use bonk_hal::Lang;
use bonk_ui::model::{Message, Model};
use bonk_ui::settings::{LangChoice, Screen, SettingsMessage, ThemeChoice};
use serde_json::{Value, json};

use crate::screen::{MODEL, lock};

/// `{"type": "settings", "action": ..., "value": ...}` from the app.
pub(crate) fn message_of(input: &Value) -> Option<Message> {
    let value = input["value"].as_str();
    let settings = match input["action"].as_str()? {
        "open" => SettingsMessage::Open,
        "close" => SettingsMessage::Close,
        "language" => SettingsMessage::Language(
            LangChoice::ALL
                .into_iter()
                .find(|choice| Some(choice.config_value()) == value)?,
        ),
        "theme" => SettingsMessage::Theme(
            ThemeChoice::ALL
                .into_iter()
                .find(|choice| Some(choice.config_value()) == value)?,
        ),
        "offline" => SettingsMessage::Offline(value? == "true"),
        "incognito_at_start" => SettingsMessage::IncognitoAtStart(value? == "true"),
        "notifications" => SettingsMessage::Notify(value? == "true"),
        _ => return None,
    };
    Some(Message::Settings(settings))
}

fn option(label: &str, action: &str, value: &str, chosen: bool) -> Value {
    json!({"label": label, "action": action, "value": value, "chosen": chosen})
}

fn choices(model: &Model) -> (Vec<Value>, Vec<Value>) {
    let lang = model.lang;
    let languages = LangChoice::ALL
        .map(|c| {
            option(
                c.label(lang),
                "language",
                c.config_value(),
                c == model.lang_choice,
            )
        })
        .into();
    let themes = ThemeChoice::ALL
        .map(|c| {
            option(
                c.label(lang),
                "theme",
                c.config_value(),
                c == model.theme_choice,
            )
        })
        .into();
    (languages, themes)
}

pub(crate) fn state(model: &Model) -> Value {
    if model.screen != Screen::Settings {
        return Value::Null;
    }
    let lang = model.lang;
    let note = model.relaunch_note();
    let (languages, themes) = choices(model);
    json!({
        "title": lang.pick("Settings", "Paramètres"),
        "back": lang.pick("Back", "Retour"),
        "enabled": model.can_relaunch(),
        "sections": [
            {"title": lang.pick("Language", "Langue"), "note": note, "options": languages},
            {"title": lang.pick("Theme", "Thème"), "note": note, "options": themes},
        ],
        "disks_title": lang.pick("Disks", "Disques"),
        "show_all": model.show_all,
        "show_all_label": lang.pick("Also show excluded disks", "Afficher aussi les disques écartés"),
        "network_title": lang.pick("Network", "Réseau"),
        "offline": model.offline,
        "offline_label": lang.pick("Offline mode", "Mode hors ligne"),
        "offline_note": lang.pick(
            "Never connect to the Internet: the catalog is hidden and only image files on this device can be written.",
            "Ne jamais se connecter à Internet : le catalogue est masqué, seules les images présentes sur cet appareil s'écrivent.",
        ),
        "incognito_at_start": model.incognito_at_start,
        "incognito_available": model.can_start_incognito(),
        "incognito_label": lang.pick("Start in incognito", "Démarrer en mode incognito"),
        "incognito_note": lang.pick(
            "From the next start, everything goes through Orbot; until it answers, nothing reaches the network. The round button still turns it off. Available once Orbot runs.",
            "Dès le prochain démarrage, tout passe par Orbot ; tant qu'il ne répond pas, rien ne part sur le réseau. Le bouton rond le coupe toujours. Disponible quand Orbot tourne.",
        ),
        "notifications_title": lang.pick("Notifications", "Notifications"),
        "notify": model.notify,
        "notify_label": lang.pick("Show the progress in the notification", "Suivre la progression dans la notification"),
        "notify_note": lang.pick(
            "Each step of a write or a download, then its end. Turned off, Android still shows that Bonk is working, without the progress.",
            "Chaque étape d'une écriture ou d'un téléchargement, puis sa fin. Coupé, Android montre toujours que Bonk travaille, sans la progression.",
        ),
    })
}

pub fn start(
    lang: &str,
    theme: &str,
    system_french: bool,
    show_all: bool,
    offline: bool,
    incognito: bool,
    notify: bool,
) {
    let mut model = lock(&MODEL);
    model.lang_choice = LangChoice::ALL
        .into_iter()
        .find(|choice| choice.config_value() == lang)
        .unwrap_or_default();
    model.lang = match model.lang_choice.config_value() {
        "fr" => Lang::Fr,
        "en" => Lang::En,
        _ if system_french => Lang::Fr,
        _ => Lang::En,
    };
    model.theme_choice = ThemeChoice::from_config(Some(theme));
    model.show_all = show_all;
    model.offline = offline;
    model.advanced |= offline;
    model.incognito_at_start = incognito;
    model.use_tor = incognito && !offline;
    model.notify = notify;
}

/// What the notification of the running write or download says, as `{title, body, percent, last}`, or null.
#[must_use]
pub fn notice() -> String {
    let model = lock(&MODEL);
    model.notice().map_or_else(
        || "null".to_owned(),
        |notice| {
            json!({
                "title": notice.title,
                "body": notice.body,
                "percent": notice.percent,
                "last": notice.last,
            })
            .to_string()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bonk_ui::command::ViewOptions;

    #[test]
    fn la_page_des_parametres_dit_ce_qui_est_choisi() {
        let mut model = Model::new(ViewOptions::default(), Lang::Fr).0;
        assert_eq!(state(&model), Value::Null);
        model.screen = Screen::Settings;
        model.theme_choice = ThemeChoice::Dark;
        let page = state(&model);
        assert_eq!(page["title"], "Paramètres");
        let chosen: Vec<&Value> = page["sections"][1]["options"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|option| option["chosen"] == true)
            .map(|option| &option["value"])
            .collect();
        assert_eq!(chosen, [&json!("dark")]);
    }

    #[test]
    fn le_reglage_des_notifications_passe_par_le_modele() {
        let input = json!({"type": "settings", "action": "notifications", "value": "false"});
        assert!(matches!(
            message_of(&input),
            Some(Message::Settings(SettingsMessage::Notify(false)))
        ));
        let mut model = Model::new(ViewOptions::default(), Lang::Fr).0;
        model.notify = true;
        model.screen = Screen::Settings;
        assert_eq!(state(&model)["notify"], true);
    }

    #[test]
    fn un_choix_inconnu_est_refuse() {
        let input = json!({"type": "settings", "action": "theme", "value": "bleu"});
        assert!(message_of(&input).is_none());
        let input = json!({"type": "settings", "action": "language", "value": "fr"});
        assert!(matches!(
            message_of(&input),
            Some(Message::Settings(SettingsMessage::Language(LangChoice::Fr)))
        ));
    }
}
