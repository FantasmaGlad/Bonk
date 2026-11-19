use super::*;

/// Every shape of the screen state: idle, draft with the menu open, writing, result, vault.
fn state_samples() -> Vec<Value> {
    let mut idle = Model::default();
    let _ = open_link(
        &mut idle,
        &json!({"type": "open", "link": "bonk://install/debian-13-netinst-x86_64"}),
    );
    let mut model = loaded();
    let mut samples = vec![state(&idle)];
    samples.extend(write_samples(&mut model));
    samples.extend(vault_samples(&mut model));
    samples.extend(catalog_samples(&mut model));
    samples
}

/// Catalog tab: grid, then a system page.
fn catalog_samples(model: &mut Model) -> Vec<Value> {
    use bonk_ui::browse::BrowseMessage;

    let _ = model.update(Message::Browse(BrowseMessage::Close));
    let _ = model.update(Message::Browse(BrowseMessage::Open));
    let mut samples = vec![state(model)];
    let _ = model.update(Message::Browse(BrowseMessage::OpenSystem(
        "Debian".to_owned(),
    )));
    samples.push(state(model));
    samples
}

/// One system with a download and an official page only.
fn debian_catalog() -> CatalogState {
    use bonk_ui::catalog::{Choice, System};

    CatalogState::Ready(vec![System {
        id: String::new(),
        name: "Debian".to_owned(),
        uses: vec!["office".to_owned(), "server".to_owned()],
        base: Some("debian".to_owned()),
        icon: None,
        choices: vec![
            Choice {
                id: "debian-13-netinst-x86_64".to_owned(),
                name: "Debian".to_owned(),
                edition: None,
                label: "13.7.0 netinst".to_owned(),
                end_of_life: false,
                note: None,
                source: ChoiceSource::Download {
                    url: "https://cdimage.debian.org/x.iso".to_owned(),
                    sha256: "ab".repeat(32),
                    file_name: None,
                },
                desktop: Some("gnome".to_owned()),
                media: "usb".to_owned(),
                arch: "x86_64".to_owned(),
            },
            Choice {
                id: "debian-13-dvd-x86_64".to_owned(),
                name: "Debian".to_owned(),
                edition: None,
                label: "DVD".to_owned(),
                end_of_life: false,
                note: Some("évaluation".to_owned()),
                source: ChoiceSource::OfficialPage("https://www.debian.org/".to_owned()),
                desktop: None,
                media: "usb".to_owned(),
                arch: "x86_64".to_owned(),
            },
        ],
    }])
}

/// Draft with the menu open and incognito on, writing, result.
fn write_samples(model: &mut Model) -> Vec<Value> {
    model.catalog = debian_catalog();
    let _ = model.update(Message::ToggleMenu);
    let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
    let _ = model.update(Message::TorDetected(Some(
        "socks5h://127.0.0.1:9050".to_owned(),
    )));
    let _ = model.update(Message::ToggleTor(true));
    // The button label counts the seconds to think: the sample wants them elapsed.
    if let WriteState::Draft { opened, .. } = &mut model.write {
        *opened = Instant::now()
            .checked_sub(Duration::from_secs(4))
            .unwrap_or(*opened);
    }
    let mut samples = vec![state(model)];
    let _ = model.update(Message::StartWrite);
    let _ = model.update(Message::Write(Event::Started(0)));
    let _ = model.update(Message::Write(Event::Progress {
        phase: Phase::Write,
        done: 1 << 20,
        total: Some(4 << 20),
        rate: 1_048_576.0,
    }));
    samples.push(state(model));
    let _ = model.update(Message::Write(Event::Finished(Outcome::Failed {
        code: Some("E2002".to_owned()),
        message: "the drive changed".to_owned(),
    })));
    samples.push(state(model));
    samples
}

/// Vault listed, then busy.
fn vault_samples(model: &mut Model) -> Vec<Value> {
    use bonk_ui::event::VaultDone;

    let _ = model.update(Message::DismissWrite);
    let _ = model.update(Message::Select("usb-S1-001-002".to_owned()));
    let _ = model.update(Message::Vault(VaultMessage::Open));
    let _ = model.update(Message::Vault(VaultMessage::Event(Event::Finished(
        Outcome::Vault(VaultDone::Listed(vec![(
            "debian-live.iso".to_owned(),
            1_500_000_000,
        )])),
    ))));
    let mut samples = vec![state(model)];
    let _ = model.update(Message::Vault(VaultMessage::Picked(Some(
        "/proc/self/fd/43".into(),
    ))));
    let _ = model.update(Message::Vault(VaultMessage::Event(Event::Progress {
        phase: Phase::Write,
        done: 1 << 20,
        total: Some(4 << 20),
        rate: 1_048_576.0,
    })));
    samples.push(state(model));
    samples
}

/// The contract between Rust and the app, written to a file the Kotlin tests read back: effects as Rust
/// emits them, messages as it accepts them. A change on one side breaks the other in CI.
/// `BONK_UPDATE_PROTOCOL=1 cargo test -p bonk-android` rewrites the file after an intended change.
#[test]
fn le_protocole_avec_kotlin_suit_ses_echantillons() -> Result<(), Box<dyn std::error::Error>> {
    let mut model = loaded();
    let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
    let effects = effect_samples(&mut model);
    let messages = message_samples();
    // `choose` and `catalog_filter` only mean something with a loaded catalog, and `open` is not a screen
    // message: their samples fix the shape only.
    for message in messages.iter().filter(|m| {
        !matches!(
            m["type"].as_str(),
            Some("choose" | "open" | "catalog_filter")
        )
    }) {
        assert!(
            message_of(message, &model).is_some(),
            "refusé par Rust : {message}"
        );
    }
    let states = state_samples();
    let text = serde_json::to_string_pretty(
        &json!({"effects": effects, "messages": messages, "states": states}),
    )? + "\n";
    check_contract(&text)
}

fn effect_samples(model: &mut Model) -> Vec<Value> {
    let draft = draft_of(model);
    effects_out(
        model,
        vec![
            Effect::Enumerate,
            Effect::LoadCatalog { proxy: None },
            Effect::LoadCatalog {
                proxy: Some("socks5h://127.0.0.1:9050".to_owned()),
            },
            Effect::PickImage,
            Effect::SaveSetting {
                key: "view.show_all",
                value: "true",
            },
            Effect::Relaunch {
                key: "theme",
                value: "dark",
            },
            Effect::StartWrite(Vec::new()),
            Effect::PickVaultIso,
            Effect::OpenLink(bonk_ui::incognito::TOR_PROJECT),
            Effect::RunVault(bonk_ui::vault::vault_args(
                VaultAction::Add,
                "usb-S1-001-002",
                Some("/proc/self/fd/43"),
                false,
                false,
            )),
            Effect::RunVault(bonk_ui::vault::vault_args(
                VaultAction::Update,
                "usb-S1-001-002",
                None,
                false,
                false,
            )),
        ],
        draft,
    )
}

fn message_samples() -> Vec<Value> {
    vec![
        json!({"type": "disks", "disks": [{
            "device": "/dev/bus/usb/001/002", "model": "Generic UDisk", "serial": "S1",
            "size_bytes": 32_000_000_000_u64,
            "partitions": [{"label": "ESP", "size_bytes": 1_048_576}, {"label": null, "size_bytes": 2_097_152}],
        }]}),
        json!({"type": "tick"}),
        json!({"type": "clock"}),
        json!({"type": "select", "id": "usb-S1-001-002"}),
        json!({"type": "details"}),
        json!({"type": "show_all", "on": true}),
        json!({"type": "settings", "action": "open"}),
        json!({"type": "settings", "action": "close"}),
        json!({"type": "settings", "action": "language", "value": "fr"}),
        json!({"type": "settings", "action": "theme", "value": "dark"}),
        json!({"type": "settings", "action": "offline", "value": "true"}),
        json!({"type": "menu"}),
        json!({"type": "category", "value": "Servers"}),
        json!({"type": "category"}),
        json!({"type": "base", "value": "Debian"}),
        json!({"type": "system", "value": "debian"}),
        json!({"type": "menu_search", "value": "kubuntu"}),
        json!({"type": "arch", "value": "aarch64 (ARM, 64 bits)"}),
        json!({"type": "arch"}),
        json!({"type": "settings", "action": "incognito_at_start", "value": "true"}),
        json!({"type": "choose", "index": 0}),
        json!({"type": "advanced"}),
        json!({"type": "image", "value": "/x.iso"}),
        json!({"type": "hash", "value": "abc"}),
        json!({"type": "raw", "on": true}),
        json!({"type": "mbr", "on": false}),
        json!({"type": "browse"}),
        json!({"type": "picked", "value": "/proc/self/fd/42"}),
        json!({"type": "start"}),
        json!({"type": "cancel"}),
        json!({"type": "dismiss"}),
        json!({"type": "download", "done": 1024, "total": 4096, "rate": 512.0}),
        json!({"type": "failed", "message": "the drive is no longer connected", "code": "E1003"}),
        json!({"type": "failed", "message": "unexpected"}),
        json!({"type": "catalog_failed", "message": "404"}),
        json!({"type": "open", "link": "bonk://install/debian-13-netinst-x86_64"}),
        json!({"type": "vault", "action": "open"}),
        json!({"type": "vault", "action": "close"}),
        json!({"type": "vault", "action": "prepare"}),
        json!({"type": "vault", "action": "confirm"}),
        json!({"type": "vault", "action": "browse"}),
        json!({"type": "vault", "action": "update"}),
        json!({"type": "vault", "action": "cancel"}),
        json!({"type": "vault_picked", "value": "/proc/self/fd/43"}),
        json!({"type": "incognito"}),
        json!({"type": "tor_project"}),
        json!({"type": "catalog", "action": "open"}),
        json!({"type": "catalog", "action": "close"}),
        json!({"type": "catalog", "action": "close_system"}),
        json!({"type": "catalog_search", "value": "deb"}),
        json!({"type": "catalog_system", "name": "Debian"}),
        json!({"type": "catalog_write", "id": "debian-13-netinst-x86_64"}),
        json!({"type": "catalog_filter", "kind": "media", "value": "Clé USB"}),
        json!({"type": "catalog_filter", "kind": "media"}),
    ]
}

fn check_contract(text: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../android/app/src/test/resources/protocole.json");
    if std::env::var_os("BONK_UPDATE_PROTOCOL").is_some() {
        std::fs::create_dir_all(path.parent().ok_or("chemin sans dossier")?)?;
        std::fs::write(&path, text)?;
    }
    let written = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        written == text,
        "{} n'est plus à jour : BONK_UPDATE_PROTOCOL=1 cargo test -p bonk-android",
        path.display()
    );
    Ok(())
}
