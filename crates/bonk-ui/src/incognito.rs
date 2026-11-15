use crate::model::{Effect, Effects, Focus, Message, Model};

/// Offered when no local Tor answers: Bonk never installs it.
pub const TOR_PROJECT: &str = "https://www.torproject.org/";

impl Model {
    /// Downloads go through Tor and the interface wears its incognito colours.
    #[must_use]
    pub fn incognito(&self) -> bool {
        self.use_tor && self.tor.is_some() && !self.offline
    }

    pub(crate) fn apply_incognito(&mut self, message: Message) -> Effects {
        match message {
            Message::TorDetected(address) => {
                self.tor = address;
                self.use_tor &= self.tor.is_some();
            }
            Message::ToggleTor(on) => {
                self.focus = Some(Focus::Tor);
                self.use_tor = on && self.tor.is_some() && !self.offline;
            }
            Message::OpenTorProject => {
                self.focus = Some(Focus::Tor);
                return vec![Effect::OpenLink(TOR_PROJECT)];
            }
            _ => {}
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCAL_TOR: &str = "socks5h://127.0.0.1:9050";

    fn with_tor() -> Model {
        let mut model = Model::default();
        let _ = model.update(Message::TorDetected(Some(LOCAL_TOR.to_owned())));
        model
    }

    #[test]
    fn sans_tor_le_bouton_n_active_rien_et_mene_au_projet() {
        let mut model = Model::default();
        let _ = model.update(Message::ToggleTor(true));
        assert!(!model.incognito());
        assert_eq!(
            model.update(Message::OpenTorProject),
            vec![Effect::OpenLink(TOR_PROJECT)]
        );
    }

    #[test]
    fn tor_detecte_s_active_au_clic_seulement() {
        let mut model = with_tor();
        assert!(!model.incognito());
        let _ = model.update(Message::ToggleTor(true));
        assert!(model.incognito());
        let _ = model.update(Message::ToggleTor(false));
        assert!(!model.incognito());
    }

    #[test]
    fn tor_disparu_coupe_l_incognito() {
        let mut model = with_tor();
        let _ = model.update(Message::ToggleTor(true));
        let _ = model.update(Message::TorDetected(None));
        assert!(!model.use_tor);
    }

    #[test]
    fn le_mode_hors_ligne_coupe_l_incognito_et_le_refuse() {
        let mut model = with_tor();
        let _ = model.update(Message::ToggleTor(true));
        let _ = model.update(Message::Settings(
            crate::settings::SettingsMessage::Offline(true),
        ));
        assert!(!model.incognito());
        let _ = model.update(Message::ToggleTor(true));
        assert!(!model.use_tor);
    }
}
