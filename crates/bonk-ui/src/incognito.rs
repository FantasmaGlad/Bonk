use crate::catalog::CatalogState;
use crate::model::{Effect, Effects, Focus, Message, Model};

/// Offered when no local Tor answers: Bonk never installs it.
pub const TOR_PROJECT: &str = "https://www.torproject.org/";

impl Model {
    /// Downloads go through Tor and the interface wears its incognito colours.
    #[must_use]
    pub fn incognito(&self) -> bool {
        self.use_tor && self.tor.is_some() && !self.offline
    }

    /// The Tor address every request goes through while incognito: catalog, logos, images.
    #[must_use]
    pub fn proxy(&self) -> Option<&str> {
        self.tor.as_deref().filter(|_| self.incognito())
    }

    pub(crate) fn load_catalog(&self) -> Effect {
        Effect::LoadCatalog {
            proxy: self.proxy().map(str::to_owned),
        }
    }

    /// Read again on the new route, unless it was never asked for; the list shown stays until the answer.
    fn reload_catalog(&self) -> Effects {
        if self.offline || matches!(self.catalog, CatalogState::NotLoaded) {
            return Vec::new();
        }
        vec![self.load_catalog()]
    }

    /// Why the catalog shown is the saved copy; nothing when it was just read.
    #[must_use]
    pub fn catalog_notice(&self) -> Option<&'static str> {
        if !self.catalog_saved || !matches!(self.catalog, CatalogState::Ready(_)) {
            return None;
        }
        Some(if self.incognito() {
            self.lang.pick(
                "Tor does not answer: the saved catalog is shown, nothing went out directly.",
                "Tor ne répond pas : le catalogue enregistré est affiché, rien n'est parti en direct.",
            )
        } else {
            self.lang.pick(
                "The catalog server does not answer: the saved catalog is shown.",
                "Le serveur du catalogue ne répond pas : le catalogue enregistré est affiché.",
            )
        })
    }

    /// Looked for at start and again at each refresh until one answers: Orbot or Tor may start after Bonk.
    pub(crate) fn watch_tor(&self) -> Option<Effect> {
        (!self.offline && self.tor.is_none()).then_some(Effect::DetectTor)
    }

    pub(crate) fn apply_incognito(&mut self, message: Message) -> Effects {
        match message {
            Message::TorDetected(address) => {
                self.tor = address;
                self.use_tor &= self.tor.is_some();
            }
            Message::ToggleTor(on) => {
                self.focus = Some(Focus::Tor);
                let before = self.incognito();
                self.use_tor = on && self.tor.is_some() && !self.offline;
                if self.incognito() != before {
                    return self.reload_catalog();
                }
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
    fn tor_est_cherche_au_depart_et_a_chaque_actualisation_tant_qu_il_manque() {
        let (mut model, effects) =
            Model::new(crate::command::ViewOptions::default(), bonk_hal::Lang::En);
        assert!(effects.contains(&Effect::DetectTor));
        model.loading = false;
        assert!(model.update(Message::Tick).contains(&Effect::DetectTor));
        let _ = model.update(Message::TorDetected(Some(LOCAL_TOR.to_owned())));
        assert!(!model.update(Message::Tick).contains(&Effect::DetectTor));
        model.offline = true;
        model.tor = None;
        assert!(!model.update(Message::Tick).contains(&Effect::DetectTor));
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

    fn catalog_read(model: &mut Model, saved_copy: bool) {
        model.catalog = CatalogState::Loading;
        let shelf = crate::catalog::Shelf {
            systems: Vec::new(),
            terms: crate::terms::Terms::default(),
            saved_copy,
        };
        let _ = model.update(Message::CatalogLoaded(Ok(shelf)));
    }

    /// Nothing leaves the machine directly once incognito: each read carries the Tor address.
    #[test]
    fn en_incognito_chaque_lecture_passe_par_tor() {
        let mut model = with_tor();
        let _ = model.update(Message::ToggleTor(true));
        assert_eq!(model.proxy(), Some(LOCAL_TOR));
        assert_eq!(
            model.request_image("debian-13-netinst-x86_64".to_owned()),
            vec![Effect::LoadCatalog {
                proxy: Some(LOCAL_TOR.to_owned())
            }]
        );
        let disk = String::from("usb-a");
        let vault = model.run_vault(disk, crate::vault::VaultAction::Prepare, None);
        assert!(
            matches!(&vault[..], [Effect::RunVault(args)] if args.ends_with(&["--proxy".to_owned(), LOCAL_TOR.to_owned()])),
            "{vault:?}"
        );
    }

    #[test]
    fn basculer_l_incognito_relit_le_catalogue_par_la_nouvelle_route() {
        let mut model = with_tor();
        assert!(model.update(Message::ToggleTor(true)).is_empty());
        catalog_read(&mut model, false);
        assert_eq!(
            model.update(Message::ToggleTor(false)),
            vec![Effect::LoadCatalog { proxy: None }]
        );
        catalog_read(&mut model, false);
        assert_eq!(
            model.update(Message::ToggleTor(true)),
            vec![Effect::LoadCatalog {
                proxy: Some(LOCAL_TOR.to_owned())
            }]
        );
        assert!(model.update(Message::ToggleTor(true)).is_empty());
    }

    #[test]
    fn la_copie_enregistree_dit_pourquoi_elle_est_affichee() {
        let mut model = with_tor();
        model.lang = bonk_hal::Lang::En;
        catalog_read(&mut model, false);
        assert_eq!(model.catalog_notice(), None);
        catalog_read(&mut model, true);
        assert!(
            model
                .catalog_notice()
                .is_some_and(|n| n.contains("catalog server"))
        );
        let _ = model.update(Message::ToggleTor(true));
        catalog_read(&mut model, true);
        assert!(
            model
                .catalog_notice()
                .is_some_and(|n| n.starts_with("Tor does not answer"))
        );
    }
}
