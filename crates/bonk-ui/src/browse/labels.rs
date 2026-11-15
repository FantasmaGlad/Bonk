use bonk_hal::Lang;

/// Desktop ids in the order the filter offers them; any other id is shown as it is.
const DESKTOPS: [&str; 19] = [
    "gnome",
    "kde",
    "xfce",
    "cinnamon",
    "mate",
    "lxqt",
    "lxde",
    "budgie",
    "cosmic",
    "pantheon",
    "deepin",
    "enlightenment",
    "i3",
    "sway",
    "unity",
    "ukui",
    "pixel",
    "choice",
    "none",
];

const MEDIA: [&str; 3] = ["usb", "sd", "optical"];

#[must_use]
pub fn desktop_label(id: &str, lang: Lang) -> String {
    match id {
        "gnome" => "GNOME",
        "kde" => "KDE Plasma",
        "xfce" => "Xfce",
        "cinnamon" => "Cinnamon",
        "mate" => "MATE",
        "lxqt" => "LXQt",
        "lxde" => "LXDE",
        "budgie" => "Budgie",
        "cosmic" => "COSMIC",
        "pantheon" => "Pantheon",
        "deepin" => "DDE",
        "enlightenment" => "Enlightenment",
        "i3" => "i3",
        "sway" => "Sway",
        "unity" => "Unity",
        "ukui" => "UKUI",
        "pixel" => lang.pick("Raspberry Pi desktop", "Bureau Raspberry Pi"),
        "none" => lang.pick("No desktop", "Sans interface"),
        "choice" => lang.pick("Desktop of your choice", "Bureau au choix"),
        other => other,
    }
    .to_owned()
}

#[must_use]
pub fn media_label(id: &str, lang: Lang) -> String {
    match id {
        "usb" => lang.pick("USB drive", "Clé USB"),
        "sd" => lang.pick("SD card", "Carte SD"),
        "optical" => lang.pick("CD or virtual machine", "CD ou machine virtuelle"),
        other => other,
    }
    .to_owned()
}

/// Known ids first, in their order, then the others sorted: a new catalog value still appears.
pub(super) fn ordered<'a>(ids: impl Iterator<Item = &'a str>, known: Order) -> Vec<String> {
    let order: &[&str] = match known {
        Order::Desktops => &DESKTOPS,
        Order::Media => &MEDIA,
    };
    let mut ids: Vec<String> = ids.map(str::to_owned).collect();
    ids.sort_by_key(|id| {
        (
            order
                .iter()
                .position(|known| known == id)
                .unwrap_or(order.len()),
            id.clone(),
        )
    });
    ids.dedup();
    ids
}

#[derive(Clone, Copy)]
pub(super) enum Order {
    Desktops,
    Media,
}
