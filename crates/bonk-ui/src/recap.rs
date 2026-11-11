use bonk_hal::{Disk, Lang, format_size};

use crate::catalog::{CatalogState, find_url};

/// What the write will do, read just before it: the image first, then the disk it erases.
#[must_use]
pub fn image_line(lang: Lang, catalog: &CatalogState, image: &str) -> String {
    let image = image.trim();
    let name = if image.is_empty() {
        lang.pick("none chosen yet", "aucune choisie pour l'instant")
            .to_owned()
    } else if let CatalogState::Ready(systems) = catalog
        && let Some((_, choice)) = find_url(systems, image)
    {
        format!("{} {}", choice.name, choice.title(lang))
    } else {
        // A path or an address: its last part is what a person recognises.
        image.rsplit(['/', '\\']).next().unwrap_or(image).to_owned()
    };
    format!("{} {name}", lang.pick("Image:", "Image :"))
}

#[must_use]
pub fn disk_line(lang: Lang, disk: &Disk) -> String {
    format!(
        "{} {}, {}",
        lang.pick("Erased disk:", "Disque effacé :"),
        disk.model,
        format_size(disk.size_bytes, lang)
    )
}

#[must_use]
pub fn warning(lang: Lang) -> &'static str {
    lang.pick(
        "All the data on this disk will be erased. This cannot be undone.",
        "Toutes les données de ce disque seront effacées. Cette action est irréversible.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_image_se_nomme_par_le_catalogue_sinon_par_son_fichier() {
        let none = CatalogState::NotLoaded;
        assert_eq!(
            image_line(Lang::Fr, &none, "  "),
            "Image : aucune choisie pour l'instant"
        );
        assert_eq!(
            image_line(Lang::En, &none, "/home/alice/images/debian-12.iso"),
            "Image: debian-12.iso"
        );
        assert_eq!(
            image_line(Lang::En, &none, "C:\\isos\\arch.iso"),
            "Image: arch.iso"
        );
    }
}
