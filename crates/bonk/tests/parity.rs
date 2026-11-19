use bonk_gui::{ViewOptions, WriteView, equivalent_commands};

const IMAGES: [&str; 4] = [
    "/tmp/debian.iso",
    "/home/u/Mes images/ubuntu 24.04.img.xz",
    "/tmp/d'artagnan; rm -rf ~.img",
    "~/téléchargements/$HOME.img",
];

#[test]
fn toutes_les_commandes_equivalentes_sont_acceptees_par_la_cli() -> Result<(), String> {
    let identifiants = [
        None,
        Some("usb-SanDisk_Cruzer_Blade_4C530001-0:0"),
        Some("nvme-eui.002538bc31bf297e"),
        Some("dev-loop0"),
        Some("clé usb avec espace"),
        Some("it's; rm -rf /"),
    ];
    let mut verifiees = 0;
    for (bits, image) in (0..8u8).flat_map(|b| IMAGES.iter().map(move |i| (b, *i))) {
        let options = ViewOptions {
            show_internal: bits & 1 != 0,
            allow_virtual: bits & 2 != 0,
            show_all: bits & 4 != 0,
            offline: false,
            incognito: false,
            notify: false,
        };
        for selection in identifiants {
            let commandes = equivalent_commands(options, selection, Some(WriteView::image(image)));
            assert_eq!(commandes.len(), 1 + 2 * usize::from(selection.is_some()));
            for commande in &commandes {
                let mots = bonk_cli::check_command_line(commande)
                    .map_err(|e| format!("{commande:?} refusée : {e}"))?;
                verifiees += 1;
                // The settings shown are exactly the view's.
                assert_eq!(
                    mots.iter().any(|m| m == "--all"),
                    options.show_all && mots[1] == "list"
                );
                assert_eq!(
                    mots.iter().any(|m| m == "--show-internal"),
                    options.show_internal
                );
                assert_eq!(
                    mots.iter().any(|m| m == "--allow-virtual"),
                    options.allow_virtual
                );
                if mots[1] == "info" || mots[1] == "write" {
                    assert_eq!(Some(mots[2].as_str()), selection, "{commande:?}");
                }
                if mots[1] == "write" {
                    // The image arrives intact, and the confirmation is part of the copied command.
                    assert_eq!(mots[3], image, "{commande:?}");
                    assert!(mots.iter().any(|m| m == "--yes"));
                }
            }
        }
    }
    assert_eq!(verifiees, 8 * IMAGES.len() * (6 + 5 * 2));
    Ok(())
}

#[test]
fn les_options_d_ecriture_de_la_fenetre_sont_acceptees_par_la_cli() -> Result<(), String> {
    let hex = "a".repeat(64);
    let cas: [(WriteView<'_>, &[&str]); 5] = [
        (WriteView::image("x.iso"), &[]),
        (
            WriteView {
                hash: &hex,
                ..WriteView::image("x.iso")
            },
            &["--sha256", &hex],
        ),
        (
            WriteView {
                hash: "https://a.org/SHA256SUMS",
                ..WriteView::image("x.iso")
            },
            &["--sha256-file", "https://a.org/SHA256SUMS"],
        ),
        (
            WriteView {
                raw: true,
                mbr: false,
                ..WriteView::image("x.img")
            },
            &["--mode", "raw", "--table", "gpt"],
        ),
        (
            WriteView {
                force: true,
                ..WriteView::image("x.iso")
            },
            &["--force"],
        ),
    ];
    for (vue, attendus) in cas {
        let commandes = equivalent_commands(ViewOptions::default(), Some("ata-a"), Some(vue));
        let mots = bonk_cli::check_command_line(&commandes[2])
            .map_err(|e| format!("{:?} refusée : {e}", commandes[2]))?;
        for attendu in attendus {
            assert!(
                mots.iter().any(|m| m == attendu),
                "{attendu} absent de {:?}",
                commandes[2]
            );
        }
    }
    Ok(())
}

#[test]
fn le_coffre_le_telechargement_et_le_cache_de_la_fenetre_sont_acceptes_par_la_cli()
-> Result<(), String> {
    use bonk_ui::vault::{VaultAction, vault_args};
    let mut lancees = Vec::new();
    for disque in [
        "usb-SanDisk_Cruzer_Blade_4C530001-0:0",
        "clé usb avec espace",
    ] {
        for (interne, virtuel) in [(false, false), (true, false), (false, true), (true, true)] {
            lancees.push(vault_args(
                VaultAction::List,
                disque,
                None,
                interne,
                virtuel,
            ));
            lancees.push(vault_args(
                VaultAction::Prepare,
                disque,
                None,
                interne,
                virtuel,
            ));
            lancees.push(vault_args(
                VaultAction::Add,
                disque,
                Some(IMAGES[1]),
                interne,
                virtuel,
            ));
            lancees.push(vault_args(
                VaultAction::Update,
                disque,
                None,
                interne,
                virtuel,
            ));
        }
    }
    lancees.push(bonk_ui::browse::download_args("ubuntu-26.04-x86_64", None));
    lancees.push(bonk_ui::browse::download_args(
        "ubuntu-26.04-x86_64",
        Some("socks5h://127.0.0.1:9050"),
    ));
    lancees.push(bonk_ui::command::cache_list_args());
    lancees.push(bonk_ui::command::cache_clean_args(&"a".repeat(64)));
    for args in &lancees {
        bonk_cli::check_args(args).map_err(|e| format!("{args:?} refusée : {e}"))?;
    }
    Ok(())
}

/// Every setting the window can save, as `Effect::SaveSetting` or `Effect::Relaunch` carries it.
fn reglages_de_la_fenetre() -> Vec<(&'static str, &'static str)> {
    use bonk_ui::model::{Effect, Message, Model};
    use bonk_ui::settings::{LangChoice, SettingsMessage, ThemeChoice};
    let mut messages = vec![
        Message::ToggleInternal(true),
        Message::ToggleVirtual(true),
        Message::ToggleAll(true),
    ];
    let reglages = [
        SettingsMessage::Offline(true),
        SettingsMessage::Offline(false),
        SettingsMessage::IncognitoAtStart(true),
        SettingsMessage::Notify(false),
        SettingsMessage::Notify(true),
    ];
    messages.extend(reglages.into_iter().map(Message::Settings));
    let mut trouves = Vec::new();
    for message in messages {
        let (mut model, _) = Model::new(ViewOptions::default(), bonk_hal::Lang::En);
        let _ = model.update(Message::Settings(SettingsMessage::Open));
        for effect in model.update(message) {
            if let Effect::SaveSetting { key, value } | Effect::Relaunch { key, value } = effect {
                trouves.push((key, value));
            }
        }
    }
    let changements = LangChoice::ALL
        .map(SettingsMessage::Language)
        .into_iter()
        .chain(ThemeChoice::ALL.map(SettingsMessage::Theme));
    for message in changements {
        let (mut model, _) = Model::new(ViewOptions::default(), bonk_hal::Lang::En);
        let _ = model.update(Message::Settings(SettingsMessage::Open));
        for effect in model.update(Message::Settings(message)) {
            if let Effect::SaveSetting { key, value } | Effect::Relaunch { key, value } = effect {
                trouves.push((key, value));
            }
        }
    }
    trouves
}

#[test]
fn chaque_reglage_de_la_fenetre_est_accepte_par_bonk_config_set()
-> Result<(), Box<dyn std::error::Error>> {
    let reglages = reglages_de_la_fenetre();
    for cle in [
        "lang",
        "theme",
        "network.offline",
        "notifications",
        "view.show_all",
        "view.show_internal",
    ] {
        assert!(
            reglages.iter().any(|(k, _)| *k == cle),
            "{cle} jamais émis : {reglages:?}"
        );
    }
    let dossier = std::env::temp_dir().join(format!("bonk-parite-{}", std::process::id()));
    std::fs::create_dir_all(&dossier)?;
    let config = dossier.join("config.json");
    for (cle, valeur) in reglages {
        let args = bonk_ui::command::config_set_args(cle, valeur);
        bonk_cli::check_args(&args)?;
        let sortie = std::process::Command::new(env!("CARGO_BIN_EXE_bonk"))
            .args(&args)
            .env("BONK_CONFIG", &config)
            .output()?;
        assert!(
            sortie.status.success(),
            "bonk config set {cle} {valeur} refusé : {}",
            String::from_utf8_lossy(&sortie.stderr)
        );
    }
    std::fs::remove_dir_all(&dossier)?;
    Ok(())
}
