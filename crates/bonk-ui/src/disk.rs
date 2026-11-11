use bonk_hal::{Disk, Lang, bus_label, format_size, kind_label};

/// What a person reads first: capacity, the names the system shows, the connection.
#[must_use]
pub fn facts(lang: Lang, disk: &Disk) -> String {
    let mut parts = vec![format_size(disk.size_bytes, lang)];
    let labels: Vec<&str> = disk
        .partitions
        .iter()
        .filter_map(|partition| partition.label.as_deref())
        .filter(|label| !label.is_empty())
        .collect();
    if !labels.is_empty() {
        parts.push(labels.join(", "));
    }
    parts.push(bus_label(disk.bus, lang).to_uppercase());
    parts.join("   ")
}

/// Device node, identifiers and partitions: shown on demand, never by default.
#[must_use]
pub fn details(lang: Lang, disk: &Disk) -> Vec<String> {
    let mut lines = vec![
        format!("{}  {}", disk.path, disk.id),
        format!(
            "{} {}   {} {}   {} {}",
            lang.pick("Serial:", "Série :"),
            disk.serial
                .as_deref()
                .unwrap_or(lang.pick("unknown", "inconnue")),
            lang.pick("Kind:", "Nature :"),
            kind_label(disk.kind, lang),
            lang.pick("System removable flag:", "Drapeau amovible du système :"),
            if disk.removable_flag {
                lang.pick("yes", "oui")
            } else {
                lang.pick("no", "non")
            },
        ),
    ];
    if disk.partitions.is_empty() {
        lines.push(
            lang.pick("No partition detected.", "Aucune partition détectée.")
                .to_owned(),
        );
    }
    lines.extend(disk.partitions.iter().map(|partition| {
        let mounts = if partition.mount_points.is_empty() {
            lang.pick("not mounted", "non monté").to_owned()
        } else {
            format!(
                "{} {}",
                lang.pick("mounted on", "monté sur"),
                partition.mount_points.join(", ")
            )
        };
        format!(
            "{}  {}  {}  {}",
            partition.path,
            format_size(partition.size_bytes, lang),
            partition
                .fs_type
                .as_deref()
                .unwrap_or(lang.pick("unknown filesystem", "système de fichiers inconnu")),
            mounts,
        )
    }));
    lines
}

/// The label of the disclosure under a selected card.
#[must_use]
pub fn details_button(lang: Lang, open: bool) -> &'static str {
    if open {
        lang.pick("Hide technical details", "Masquer les détails techniques")
    } else {
        lang.pick("Technical details", "Détails techniques")
    }
}

#[cfg(test)]
mod tests {
    use bonk_hal::{Bus, DiskKind, Partition};

    use super::*;

    fn key() -> Disk {
        Disk {
            id: "usb-Mass_Storage_Device_121220160204".to_owned(),
            path: "/dev/sda".to_owned(),
            model: "Mass Storage Device".to_owned(),
            serial: Some("121220160204".to_owned()),
            size_bytes: 255_800_000_000,
            bus: Bus::Usb,
            kind: DiskKind::Physical,
            removable_flag: true,
            logical_block_size: None,
            location: None,
            fs_type: None,
            holders: Vec::new(),
            partitions: vec![Partition {
                path: "/dev/sda1".to_owned(),
                label: Some("BONK-WIN".to_owned()),
                fs_type: Some("exfat".to_owned()),
                size_bytes: 255_000_000_000,
                mount_points: vec!["/run/media/fanta/BONK-WIN".to_owned()],
                holders: Vec::new(),
                is_system_volume: false,
                is_active_esp: false,
            }],
        }
    }

    /// The card says what a person recognises; nodes, serials and mounts wait in the details.
    #[test]
    fn la_carte_est_lisible_et_les_details_gardent_le_technique() {
        let disk = key();
        let facts = facts(Lang::En, &disk);
        assert!(
            facts.contains("BONK-WIN") && facts.contains("USB"),
            "{facts}"
        );
        for technical in ["/dev/sda", "121220160204", "/run/media"] {
            assert!(!facts.contains(technical), "{technical} : {facts}");
        }
        let details = details(Lang::En, &disk).join("\n");
        for technical in ["/dev/sda1", "121220160204", "/run/media/fanta/BONK-WIN"] {
            assert!(details.contains(technical), "{technical}");
        }
    }
}
