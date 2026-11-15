use bonk_core::partition::Layout;
use bonk_hal::{Bus, Disk, DiskKind, Partition, state_label, verdict_badge};
use bonk_ui::model::Model;
use serde_json::{Value, json};

/// As the safety filter judges it. Android only shows USB: neither the phone's internal storage nor its
/// partitions, which the system keeps.
pub(super) fn disk_of(value: &Value) -> Option<Disk> {
    let serial = value["serial"].as_str().filter(|s| !s.is_empty());
    let name = value["device"].as_str()?;
    // The serial alone is not enough: cloned keys share it, and the tapped card would then name the first in
    // the list. The plug point tells them apart; replugging the key changes its id, and the draft starts over
    // (empty image, new delay) instead of targeting whatever took the place.
    let port = name
        .strip_prefix("/dev/bus/usb/")
        .unwrap_or(name)
        .replace('/', "-");
    Some(Disk {
        id: match serial {
            Some(serial) => format!("usb-{serial}-{port}"),
            None => format!("usb-{port}"),
        },
        location: Some(port.clone()),
        path: name.to_owned(),
        model: value["model"].as_str()?.to_owned(),
        serial: serial.map(str::to_owned),
        size_bytes: value["size_bytes"].as_u64()?,
        bus: Bus::Usb,
        kind: DiskKind::Physical,
        removable_flag: true,
        logical_block_size: None,
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

/// Without them the summary before erasing would show a full key as empty. GPT first, else MBR without the
/// protective entry.
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

/// As [`partitions_of`] reads them, in the shape the safety filter judges.
pub(crate) fn partitions(layout: &Layout) -> Vec<Partition> {
    partitions_of(layout)
        .iter()
        .enumerate()
        .filter_map(partition_of)
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

pub(super) fn disks_state(model: &Model) -> Vec<Value> {
    let lang = model.lang;
    model
        .entries()
        .iter()
        .map(|(disk, verdict)| {
            let selected = model.selected.as_deref() == Some(disk.id.as_str());
            json!({
                "id": disk.id,
                "device": disk.path,
                "model": disk.model,
                "facts": bonk_ui::disk::facts(lang, disk),
                "badge": verdict_badge(verdict, lang),
                "state": state_label(verdict, lang),
                "selected": selected,
                // Device node, serial and partitions: on demand, under the selected card.
                "details": if selected && model.details {
                    bonk_ui::disk::details(lang, disk)
                } else {
                    Vec::new()
                },
                "details_button": selected.then(|| bonk_ui::disk::details_button(lang, model.details)),
            })
        })
        .collect()
}
