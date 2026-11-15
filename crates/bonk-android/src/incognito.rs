use bonk_ui::incognito::TOR_PROJECT;
use bonk_ui::model::{Message, Model};
use serde_json::{Value, json};

/// The header's round button; `null` offline, where nothing is downloaded.
pub(crate) fn state(model: &Model) -> Value {
    if model.offline {
        return Value::Null;
    }
    let lang = model.lang;
    let label = match (&model.tor, model.use_tor) {
        (None, _) => lang.pick("Incognito: no Tor", "Incognito : Tor absent"),
        (Some(_), true) => lang.pick("Incognito on (Tor)", "Incognito activé (Tor)"),
        (Some(_), false) => lang.pick("Incognito off", "Incognito désactivé"),
    };
    json!({
        "available": model.tor.is_some(),
        "on": model.incognito(),
        "label": label,
        "link": TOR_PROJECT.trim_start_matches("https://www.").trim_end_matches('/'),
    })
}

pub(crate) fn message_of(input: &Value, model: &Model) -> Option<Message> {
    Some(match input["type"].as_str()? {
        "incognito" => Message::ToggleTor(!model.use_tor),
        "tor_project" => Message::OpenTorProject,
        _ => return None,
    })
}

/// Orbot's SOCKS port, for the downloads of the next write; the address keeps `socks5h` so names are
/// resolved by Tor.
pub(crate) fn proxy(model: &Model) -> Option<&str> {
    model.tor.as_deref().filter(|_| model.incognito())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORBOT: &str = "socks5h://127.0.0.1:9050";

    #[test]
    fn sans_tor_le_bouton_est_inactif_et_donne_le_lien() {
        let model = Model::default();
        let shown = state(&model);
        assert_eq!(shown["available"], false);
        assert_eq!(shown["link"], "torproject.org");
        assert_eq!(proxy(&model), None);
    }

    fn touch(model: &mut Model) {
        if let Some(message) = message_of(&json!({"type": "incognito"}), model) {
            let _ = model.update(message);
        }
    }

    #[test]
    fn le_bouton_bascule_tor_et_le_proxy_suit() {
        let mut model = Model::default();
        let _ = model.update(Message::TorDetected(Some(ORBOT.to_owned())));
        touch(&mut model);
        assert_eq!(state(&model)["on"], true);
        assert_eq!(proxy(&model), Some(ORBOT));
        touch(&mut model);
        assert_eq!(proxy(&model), None);
    }

    #[test]
    fn hors_ligne_le_bouton_disparait() {
        let model = Model {
            offline: true,
            ..Model::default()
        };
        assert_eq!(state(&model), Value::Null);
    }
}
