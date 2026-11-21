use crate::catalog::CatalogState;
use crate::model::{Effect, Effects, Focus, Message, Model};

/// Offered when no local Tor answers: Bonk never installs it.
pub const TOR_PROJECT: &str = "https://www.torproject.org/";

impl Model {
    #[must_use]
    pub fn incognito(&self) -> bool {
        self.use_tor && self.tor.is_some() && !self.offline
    }

    /// Incognito asked for while no Tor answers yet: nothing may reach the network meanwhile.
    #[must_use]
    pub fn tor_awaited(&self) -> bool {
        self.use_tor && self.tor.is_none() && !self.offline
    }

    #[must_use]
    pub fn network_closed(&self) -> bool {
        self.offline || self.tor_awaited()
    }

    #[must_use]
    pub fn proxy(&self) -> Option<&str> {
        self.tor.as_deref().filter(|_| self.incognito())
    }

    pub(crate) fn load_catalog(&self) -> Effect {
        Effect::LoadCatalog {
            proxy: self.proxy().map(str::to_owned),
        }
    }

    /// The only way the catalog is asked for: never before Tor when incognito is waiting for it.
    pub(crate) fn read_catalog(&mut self) -> Effects {
        if self.tor_awaited() {
            self.catalog = CatalogState::Failed(
                self.lang
                    .pick(
                        "Waiting for Tor: incognito keeps everything off the network until it answers.",
                        "En attente de Tor : l'incognito ne laisse rien partir sur le réseau avant sa réponse.",
                    )
                    .to_owned(),
            );
            return Vec::new();
        }
        self.catalog = CatalogState::Loading;
        vec![self.load_catalog()]
    }

    fn reload_catalog(&mut self) -> Effects {
        match self.catalog {
            _ if self.offline => Vec::new(),
            CatalogState::NotLoaded => Vec::new(),
            CatalogState::Failed(_) => self.read_catalog(),
            _ => vec![self.load_catalog()],
        }
    }

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

    /// Turned on only with a Tor answering now: a setting nothing could honour would hold every start.
    #[must_use]
    pub fn can_start_incognito(&self) -> bool {
        self.incognito_at_start || self.tor.is_some()
    }

    #[must_use]
    pub fn incognito_label(&self) -> &'static str {
        match (&self.tor, self.use_tor) {
            (None, true) => self.lang.pick(
                "Incognito: waiting for Tor",
                "Incognito : en attente de Tor",
            ),
            (None, false) => self.lang.pick(
                "Incognito: no Tor running. Open Tor Browser and connect, or start Orbot or the tor service.",
                "Incognito : aucun Tor en marche. Ouvrez le navigateur Tor et connectez-le, ou lancez Orbot ou le service tor.",
            ),
            (Some(_), true) => self
                .lang
                .pick("Incognito on (Tor)", "Incognito activé (Tor)"),
            (Some(_), false) => self.lang.pick("Incognito off", "Incognito désactivé"),
        }
    }

    /// Looked for at start and again at each refresh until one answers: Orbot or Tor may start after Bonk.
    pub(crate) fn watch_tor(&self) -> Option<Effect> {
        (!self.offline && self.tor.is_none()).then_some(Effect::DetectTor)
    }

    pub(crate) fn apply_incognito(&mut self, message: Message) -> Effects {
        match message {
            Message::TorDetected(address) => {
                let awaited = self.tor_awaited();
                self.tor = address;
                if !self.incognito_at_start {
                    self.use_tor &= self.tor.is_some();
                }
                if awaited && self.incognito() {
                    return self.reload_catalog();
                }
            }
            Message::ToggleTor(on) => {
                self.focus = Some(Focus::Tor);
                let before = (self.incognito(), self.tor_awaited());
                // Turning it off is always possible, even while Tor is awaited.
                self.use_tor = on && self.tor.is_some() && !self.offline;
                if (self.incognito(), self.tor_awaited()) != before {
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

    fn started_incognito() -> Model {
        let options = crate::command::ViewOptions {
            incognito: true,
            notify: false,
            ..crate::command::ViewOptions::default()
        };
        let (mut model, _) = Model::new(options, bonk_hal::Lang::En);
        model.loading = false;
        model
    }

    #[test]
    fn au_demarrage_rien_ne_part_avant_tor() {
        let mut model = started_incognito();
        let _ = model.update(Message::TorDetected(None));
        assert!(model.tor_awaited() && model.network_closed());
        assert!(
            model
                .request_image("debian-13-netinst-x86_64".to_owned())
                .is_empty()
        );
        assert!(matches!(model.catalog, CatalogState::Failed(_)));
        assert!(model.refuses_address("https://exemple.org/a.iso"));
        let vault = model.run_vault("usb-a".to_owned(), crate::vault::VaultAction::Prepare, None);
        assert!(vault.is_empty());
        assert_eq!(model.incognito_label(), "Incognito: waiting for Tor");
        assert_eq!(
            model.update(Message::TorDetected(Some(LOCAL_TOR.to_owned()))),
            vec![Effect::LoadCatalog {
                proxy: Some(LOCAL_TOR.to_owned())
            }]
        );
        assert!(model.incognito() && !model.network_closed());
    }

    #[test]
    fn l_attente_de_tor_se_coupe_au_bouton() {
        let mut model = started_incognito();
        let _ = model.request_image("debian-13-netinst-x86_64".to_owned());
        assert_eq!(
            model.update(Message::ToggleTor(false)),
            vec![Effect::LoadCatalog { proxy: None }]
        );
        assert!(!model.network_closed());
    }

    #[test]
    fn le_reglage_se_garde_pour_les_demarrages_suivants() {
        let mut model = with_tor();
        assert_eq!(
            model.update(Message::Settings(
                crate::settings::SettingsMessage::IncognitoAtStart(true)
            )),
            vec![Effect::SaveSetting {
                key: "network.incognito",
                value: "true",
            }]
        );
        assert!(model.incognito_at_start && !model.use_tor);
        assert!(model.view_options().incognito);
    }

    #[test]
    fn demarrer_en_incognito_exige_un_tor_qui_repond() {
        let set = |model: &mut Model, on| {
            model.update(Message::Settings(
                crate::settings::SettingsMessage::IncognitoAtStart(on),
            ))
        };
        let mut model = Model::default();
        assert!(!model.can_start_incognito());
        assert!(set(&mut model, true).is_empty());
        assert!(!model.incognito_at_start);
        let mut model = with_tor();
        assert!(!set(&mut model, true).is_empty());
        let _ = model.update(Message::TorDetected(None));
        assert!(model.can_start_incognito(), "le couper reste possible");
        assert!(!set(&mut model, false).is_empty());
        assert!(!model.incognito_at_start);
    }
}
