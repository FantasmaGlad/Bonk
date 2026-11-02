//! L'écran de l'application : le modèle commun à toutes les interfaces, vu depuis Kotlin. Un message entre en
//! JSON, l'état à dessiner et les effets à accomplir en sortent. Kotlin dessine, il ne décide rien.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};

use bonk_core::partition::Layout;
use bonk_core::write::Phase;
use bonk_hal::{Bus, Disk, DiskKind, Lang, Partition, format_size, state_label, verdict_badge};
use bonk_ui::catalog::{CatalogState, ChoiceSource, all_categories, all_systems, writable};
use bonk_ui::command::ViewOptions;
use bonk_ui::event::{Event, Outcome};
use bonk_ui::model::{Effect, Message, Model, WRITE_DELAY, WriteState};
use bonk_ui::text;
use serde_json::{Value, json};

pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);
/// Ce que rapporte l'écriture, en attendant le prochain passage de l'écran.
pub(crate) static EVENTS: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());
pub(crate) static MODEL: LazyLock<Mutex<Model>> =
    LazyLock::new(|| Mutex::new(Model::new(ViewOptions::default(), Lang::En).0));

/// Le disque tel que le filtre de sécurité le juge. Android ne montre que de l'USB : ni le stockage interne
/// du téléphone, ni ses partitions, que le système garde pour lui.
fn disk_of(value: &Value) -> Option<Disk> {
    let serial = value["serial"].as_str().filter(|s| !s.is_empty());
    let name = value["device"].as_str()?;
    // La série seule ne suffit pas : des clés clones la partagent, et la carte touchée désignerait alors la
    // première de la liste. Le point de branchement la départage ; rebrancher la clé change donc son
    // identifiant, et le brouillon repart de zéro (image vide, nouveau délai) au lieu de viser ce qui
    // occupe la place.
    let port = name
        .strip_prefix("/dev/bus/usb/")
        .unwrap_or(name)
        .replace('/', "-");
    Some(Disk {
        id: match serial {
            Some(serial) => format!("usb-{serial}-{port}"),
            None => format!("usb-{port}"),
        },
        path: name.to_owned(),
        model: value["model"].as_str()?.to_owned(),
        serial: serial.map(str::to_owned),
        size_bytes: value["size_bytes"].as_u64()?,
        bus: Bus::Usb,
        kind: DiskKind::Physical,
        removable_flag: true,
        fs_type: None,
        holders: Vec::new(),
        partitions: value["partitions"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(partition_of)
            .collect(),
    })
}

/// Les partitions lues sur la clé, telles que la sonde les rend à l'application : sans elles, le récapitulatif
/// avant effacement présenterait une clé pleine comme vide. GPT d'abord ; sinon MBR hors table protectrice.
pub(crate) fn partitions_of(layout: &Layout) -> Vec<Value> {
    let block = u64::from(layout.logical_block_size);
    if let Some(gpt) = layout.gpt.as_ref().filter(|gpt| !gpt.entries.is_empty()) {
        return gpt
            .entries
            .iter()
            .map(|entry| {
                json!({
                    "label": (!entry.name.is_empty()).then(|| entry.name.clone()),
                    "size_bytes": (entry.last_lba.saturating_sub(entry.first_lba) + 1).saturating_mul(block),
                })
            })
            .collect();
    }
    layout
        .mbr
        .iter()
        .filter(|mbr| mbr.signature_valid && !mbr.protective)
        .flat_map(|mbr| &mbr.entries)
        .filter(|entry| entry.kind != 0)
        .map(|entry| json!({"label": null, "size_bytes": u64::from(entry.sectors).saturating_mul(block)}))
        .collect()
}

fn partition_of((index, value): (usize, &Value)) -> Option<Partition> {
    Some(Partition {
        path: format!("{}", index + 1),
        label: value["label"].as_str().map(str::to_owned),
        fs_type: None,
        size_bytes: value["size_bytes"].as_u64()?,
        mount_points: Vec::new(),
        holders: Vec::new(),
        is_system_volume: false,
        is_active_esp: false,
    })
}

fn message_of(input: &Value, model: &Model) -> Option<Message> {
    let flag = || input["on"].as_bool().unwrap_or(false);
    let text_of = |key: &str| input[key].as_str().map(str::to_owned);
    Some(match input["type"].as_str()? {
        "disks" => Message::Loaded(Ok(input["disks"]
            .as_array()?
            .iter()
            .filter_map(disk_of)
            .collect())),
        "tick" => Message::Tick,
        "clock" => Message::Clock,
        "select" => Message::Select(text_of("id")?),
        "show_all" => Message::ToggleAll(flag()),
        "language" => Message::SwitchLanguage,
        "menu" => Message::ToggleMenu,
        "category" => Message::MenuCategory(text_of("value")),
        "system" => Message::MenuSystem(text_of("value")),
        "catalog_failed" => Message::CatalogLoaded(Err(text_of("message")?)),
        "choose" => {
            let CatalogState::Ready(systems) = &model.catalog else {
                return None;
            };
            let index = usize::try_from(input["index"].as_u64()?).ok()?;
            let choice = writable(&model.menu_filter.apply(systems))
                .get(index)
                .copied()?;
            Message::MenuChose(choice.clone())
        }
        "advanced" => Message::ToggleAdvanced,
        "image" => Message::ImageInput(text_of("value")?),
        "hash" => Message::HashInput(text_of("value")?),
        "raw" => Message::ToggleRaw(flag()),
        "mbr" => Message::ToggleMbr(flag()),
        "browse" => Message::BrowseImage,
        "picked" => Message::ImagePicked(text_of("value").map(Into::into)),
        "start" => Message::StartWrite,
        "confirm" => Message::ConfirmInternal,
        "close_confirm" => Message::CloseInternal,
        "cancel" => Message::CancelWrite,
        "dismiss" => Message::DismissWrite,
        "download" => Message::Write(Event::Progress {
            phase: Phase::Download,
            done: input["done"].as_u64()?,
            total: input["total"].as_u64(),
            rate: input["rate"].as_f64().unwrap_or(0.0),
        }),
        "failed" => Message::Write(Event::Finished(Outcome::Failed {
            code: None,
            message: text_of("message")?,
        })),
        _ => return None,
    })
}

/// Ce que Kotlin doit faire. Ce que Rust sait faire seul ne sort pas d'ici.
fn effects_out(model: &mut Model, effects: Vec<Effect>, draft: Option<Value>) -> Vec<Value> {
    let mut out = Vec::new();
    for effect in effects {
        match effect {
            Effect::Enumerate => out.push(json!({"do": "enumerate"})),
            Effect::LoadCatalog => out.push(json!({"do": "catalog"})),
            Effect::PickImage => out.push(json!({"do": "pick"})),
            Effect::SaveLanguage(lang) => out.push(json!({
                "do": "save_language",
                "value": if lang == Lang::Fr { "fr" } else { "en" },
            })),
            // Tor n'existe pas sur le téléphone sans application dédiée : l'option reste éteinte.
            Effect::DetectTor => {
                let _ = model.update(Message::TorDetected(None));
            }
            Effect::CancelWrite(_) => CANCEL.store(true, Ordering::SeqCst),
            Effect::StartWrite(_) => {
                CANCEL.store(false, Ordering::SeqCst);
                let _ = model.update(Message::Write(Event::Started(0)));
                if let Some(mut write) = draft.clone() {
                    write["do"] = json!("write");
                    out.push(write);
                }
            }
            Effect::Copy(_) | Effect::ScrollList(_) | Effect::ScrollMenu(_) => {}
        }
    }
    out
}

fn draft_of(model: &Model) -> Option<Value> {
    let WriteState::Draft {
        disk_id,
        image,
        hash,
        raw,
        mbr,
        ..
    } = &model.write
    else {
        return None;
    };
    let device = model
        .disks
        .iter()
        .find(|disk| &disk.id == disk_id)
        .map(|disk| disk.path.clone());
    Some(json!({
        "disk_id": disk_id,
        "device": device,
        "image": image.trim(),
        "hash": hash.trim(),
        "raw": raw,
        "mbr": mbr,
    }))
}

fn disks_state(model: &Model) -> Vec<Value> {
    let lang = model.lang;
    model
        .entries()
        .iter()
        .map(|(disk, verdict)| {
            json!({
                "id": disk.id,
                "device": disk.path,
                "model": disk.model,
                "facts": format!(
                    "{}  {}  {}",
                    format_size(disk.size_bytes, lang),
                    disk.serial.as_deref().unwrap_or(lang.pick("no serial", "sans série")),
                    disk.path,
                ),
                "badge": verdict_badge(verdict, lang),
                "state": state_label(verdict, lang),
                "selected": model.selected.as_deref() == Some(disk.id.as_str()),
            })
        })
        .collect()
}

fn menu_state(model: &Model) -> Value {
    let lang = model.lang;
    let CatalogState::Ready(systems) = &model.catalog else {
        let line = match &model.catalog {
            CatalogState::Failed(error) => error.clone(),
            _ => lang
                .pick(
                    "Reading the list of systems...",
                    "Lecture de la liste des systèmes...",
                )
                .to_owned(),
        };
        return json!({"ready": false, "line": line});
    };
    let mut categories: Vec<String> = systems.iter().map(|s| s.category.clone()).collect();
    categories.sort();
    categories.dedup();
    let filtered = model.menu_filter.apply(systems);
    let mut names: Vec<String> = bonk_ui::catalog::MenuFilter {
        category: model.menu_filter.category.clone(),
        system: None,
    }
    .apply(systems)
    .iter()
    .map(|s| s.name.clone())
    .collect();
    names.sort();
    let mut index = 0;
    let mut rows = Vec::new();
    // Liste longue : la fabriquer et la relire à chaque battement ralentirait tout l'écran pour rien.
    let shown: &[&bonk_ui::catalog::System] = if model.menu_open { &filtered } else { &[] };
    for system in shown {
        for choice in &system.choices {
            let usable = matches!(choice.source, ChoiceSource::Download { .. });
            let note = match &choice.source {
                ChoiceSource::OfficialPage(page) => Some(match lang {
                    Lang::En => format!("download from {page}, then Other image"),
                    Lang::Fr => format!("à télécharger sur {page}, puis Autre image"),
                }),
                _ => choice.note.clone(),
            };
            rows.push(json!({
                "title": format!("{} {}", choice.name, choice.title(lang)),
                "note": note,
                "index": usable.then_some(index),
            }));
            index += usize::from(usable);
        }
    }
    json!({
        "ready": true,
        "open": model.menu_open,
        "button": chosen_title(model).unwrap_or_else(|| lang.pick("Choose a system...", "Choisir un système...").to_owned()),
        "all_categories": all_categories(lang),
        "all_systems": all_systems(lang),
        "category": model.menu_filter.category,
        "system": model.menu_filter.system,
        "categories": categories,
        "systems": names,
        "rows": rows,
    })
}

fn chosen_title(model: &Model) -> Option<String> {
    let (CatalogState::Ready(systems), WriteState::Draft { image, .. }) =
        (&model.catalog, &model.write)
    else {
        return None;
    };
    bonk_ui::catalog::find_url(systems, image)
        .map(|(_, choice)| format!("{} {}", choice.name, choice.title(model.lang)))
}

fn panel_state(model: &Model) -> Value {
    let lang = model.lang;
    match &model.write {
        WriteState::Idle => Value::Null,
        WriteState::Draft {
            disk_id,
            image,
            hash,
            raw,
            mbr,
            opened,
        } => {
            let elapsed = opened.elapsed();
            let empty = image.trim().is_empty();
            let disk = model.disks.iter().find(|disk| &disk.id == disk_id);
            json!({
                "kind": "draft",
                "title": lang.pick("WRITE AN IMAGE TO THIS DISK", "ÉCRIRE UNE IMAGE SUR CE DISQUE"),
                "summary": disk.map(|disk| text::disk_summary(lang, disk)),
                "warning": lang.pick(
                    "All the data on this disk will be erased. This cannot be undone.",
                    "Toutes les données de ce disque seront effacées. Cette action est irréversible.",
                ),
                "menu": menu_state(model),
                "advanced": model.advanced,
                "advanced_button": if model.advanced {
                    lang.pick("Hide the manual image and options", "Masquer l'image manuelle et les options")
                } else {
                    lang.pick("Other image or options...", "Autre image ou options...")
                },
                "image": image,
                "image_hint": lang.pick("https:// address of the image", "Adresse https:// de l'image"),
                "pick": lang.pick("Choose the image...", "Choisir l'image..."),
                "hash": hash,
                "hash_hint": lang.pick("SHA-256 hash (required for an address)", "Empreinte SHA-256 (exigée pour une adresse)"),
                "raw": raw,
                "raw_label": lang.pick(
                    "Write the bytes as they are, without refusing a non-bootable image",
                    "Écrire les octets tels quels, sans refuser une image non amorçable",
                ),
                "mbr": mbr,
                "mbr_label": lang.pick(
                    "Windows drive (extraction, FAT32, UEFI boot): MBR table instead of GPT",
                    "Clé Windows (extraction, FAT32, démarrage UEFI) : table MBR au lieu de GPT",
                ),
                "button": text::write_button(lang, elapsed, empty),
                "enabled": elapsed >= WRITE_DELAY && !empty,
            })
        }
        WriteState::Running {
            phase,
            done,
            total,
            rate,
            ..
        } => {
            let (line, fraction) = text::progress(lang, *phase, *done, *total, *rate);
            json!({
                "kind": "running",
                "line": line,
                "fraction": fraction,
                "cancel": lang.pick("Cancel", "Annuler"),
            })
        }
        WriteState::Finished { outcome, .. } => json!({
            "kind": "finished",
            "message": text::outcome(lang, outcome),
            "close": lang.pick("Close", "Fermer"),
        }),
    }
}

/// Tout ce que l'écran affiche.
#[must_use]
pub fn state(model: &Model) -> Value {
    let lang = model.lang;
    json!({
        "intro": lang.pick(
            "Disk detection. A write needs an offered disk, a summary and three seconds to think.",
            "Détection des disques. Une écriture exige un disque proposé, un récapitulatif et trois secondes de réflexion.",
        ),
        "language": lang.pick("Français", "English"),
        "status": model.status_line(model.entries().len()),
        "show_all": model.show_all,
        "show_all_label": lang.pick("Also show excluded disks", "Afficher aussi les disques écartés"),
        "empty": lang.pick(
            "No disk offered. Plug in a USB drive or an SD card with an adapter.",
            "Aucun disque proposé. Branchez une clé USB ou une carte SD avec un adaptateur.",
        ),
        "disks": disks_state(model),
        "panel": panel_state(model),
        "running": matches!(model.write, WriteState::Running { .. }),
    })
}

/// Un message, ou seulement un regard sur l'état avec `{}` : l'état et les effets à accomplir.
#[must_use]
pub fn dispatch(input: &str) -> String {
    let Ok(mut model) = MODEL.lock() else {
        return json!({"error": "state unavailable"}).to_string();
    };
    let input: Value = serde_json::from_str(input).unwrap_or_default();
    let mut effects = Vec::new();
    let pending: Vec<Event> = EVENTS
        .lock()
        .map(|mut events| events.drain(..).collect())
        .unwrap_or_default();
    for event in pending {
        effects.extend(model.update(Message::Write(event)));
    }
    let draft = draft_of(&model);
    if let Some(message) = message_of(&input, &model) {
        effects.extend(model.update(message));
    }
    let effects = effects_out(&mut model, effects, draft);
    json!({"state": state(&model), "effects": effects}).to_string()
}

/// Pose la langue de départ, celle du système ou celle choisie la fois d'avant.
pub fn set_language(fr: bool) {
    if let Ok(mut model) = MODEL.lock() {
        model.lang = if fr { Lang::Fr } else { Lang::En };
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn usb() -> Value {
        json!({"device": "/dev/bus/usb/001/002", "model": "UDisk", "serial": "S1", "size_bytes": 32_000_000_000_u64})
    }

    fn loaded() -> Model {
        let mut model = Model {
            auto_select: true,
            ..Model::default()
        };
        let disks: Vec<Disk> = disk_of(&usb()).into_iter().collect();
        let _ = model.update(Message::Loaded(Ok(disks)));
        model
    }

    #[test]
    fn la_cle_passe_le_meme_filtre_que_sur_pc_et_se_selectionne() {
        let model = loaded();
        assert_eq!(model.selected.as_deref(), Some("usb-S1-001-002"));
        let state = state(&model);
        assert_eq!(state["disks"][0]["model"], "UDisk");
        assert_eq!(state["panel"]["kind"], "draft");
    }

    #[test]
    fn le_bouton_attend_l_image_et_les_trois_secondes() {
        let mut model = loaded();
        assert_eq!(state(&model)["panel"]["enabled"], false);
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        assert_eq!(
            state(&model)["panel"]["enabled"],
            false,
            "délai de réflexion"
        );
        if let WriteState::Draft { opened, .. } = &mut model.write {
            *opened = Instant::now()
                .checked_sub(Duration::from_secs(4))
                .unwrap_or(*opened);
        }
        assert_eq!(state(&model)["panel"]["enabled"], true);
    }

    #[test]
    fn un_depart_demande_a_kotlin_d_ecrire_avec_le_brouillon() {
        let mut model = loaded();
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        if let WriteState::Draft { opened, .. } = &mut model.write {
            *opened = Instant::now()
                .checked_sub(Duration::from_secs(4))
                .unwrap_or(*opened);
        }
        let draft = draft_of(&model);
        let effects = model.update(Message::StartWrite);
        let out = effects_out(&mut model, effects, draft);
        assert_eq!(out[0]["do"], "write");
        assert_eq!(out[0]["device"], "/dev/bus/usb/001/002");
        assert!(matches!(model.write, WriteState::Running { .. }));
    }

    /// Une ligne qui change de hauteur à chaque détection fait sauter tout l'écran.
    #[test]
    fn la_detection_periodique_ne_change_pas_l_etat_affiche() {
        let mut model = Model {
            auto_select: true,
            ..Model::default()
        };
        let _ = model.update(Message::Loaded(Ok(Vec::new())));
        let before = state(&model);
        let _ = model.update(Message::Tick);
        assert!(model.loading);
        assert_eq!(state(&model), before);
        let _ = model.update(Message::Loaded(Ok(Vec::new())));
        assert_eq!(state(&model), before);
    }

    /// Deux clés clones ont souvent la même série : chacune doit garder sa propre carte, et toucher la
    /// seconde doit faire écrire la seconde, jamais la première de la liste.
    #[test]
    fn deux_cles_de_meme_serie_restent_distinctes() {
        let mut model = Model {
            auto_select: false,
            ..Model::default()
        };
        let mut b = usb();
        b["device"] = json!("/dev/bus/usb/001/003");
        let disks: Vec<Disk> = [usb(), b].iter().filter_map(disk_of).collect();
        let _ = model.update(Message::Loaded(Ok(disks)));
        let shown = disks_state(&model);
        assert_eq!(shown.len(), 2);
        assert_ne!(shown[0]["id"], shown[1]["id"]);
        let second = shown
            .iter()
            .find(|disk| disk["device"] == "/dev/bus/usb/001/003")
            .and_then(|disk| disk["id"].as_str())
            .map(str::to_owned)
            .unwrap_or_default();
        let _ = model.update(Message::Select(second));
        let selected: Vec<Value> = disks_state(&model)
            .into_iter()
            .filter(|disk| disk["selected"] == true)
            .map(|disk| disk["device"].clone())
            .collect();
        assert_eq!(selected, vec![json!("/dev/bus/usb/001/003")]);
        let _ = model.update(Message::ImageInput("/x.iso".to_owned()));
        assert_eq!(
            draft_of(&model).map(|draft| draft["device"].clone()),
            Some(json!("/dev/bus/usb/001/003"))
        );
    }

    /// Le récapitulatif avant effacement dit ce que contient la clé : une table lue, jamais « 0 » par défaut.
    #[test]
    fn le_recapitulatif_compte_les_partitions_lues_sur_la_cle()
    -> Result<(), Box<dyn std::error::Error>> {
        use bonk_core::MemoryDevice;
        use bonk_core::format::table::{TableKind, write_table};
        use bonk_core::format::{Filesystem, Ids};

        let mut dev = MemoryDevice::new(64 << 20, 512)?;
        write_table(
            &mut dev,
            TableKind::Gpt,
            Filesystem::Fat32,
            Some("DONNEES"),
            &Ids::from_seed(b"cle"),
        )?;
        let partitions = partitions_of(&bonk_core::partition::inspect(&dev)?);
        assert_eq!(partitions.len(), 1);
        let mut value = usb();
        value["partitions"] = json!(partitions);
        let mut model = Model {
            auto_select: true,
            ..Model::default()
        };
        let _ = model.update(Message::Loaded(Ok(disk_of(&value).into_iter().collect())));
        let recap = state(&model)["panel"]["summary"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(recap.contains("1 partition(s)"), "{recap}");
        // Une clé vierge n'a vraiment aucune partition.
        let blank = MemoryDevice::new(64 << 20, 512)?;
        assert!(partitions_of(&bonk_core::partition::inspect(&blank)?).is_empty());
        Ok(())
    }

    #[test]
    fn une_cle_de_taille_inconnue_n_est_pas_un_disque() {
        assert!(disk_of(&json!({"device": "d", "model": "m"})).is_none());
    }
}
