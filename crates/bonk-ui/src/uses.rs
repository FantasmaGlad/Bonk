use bonk_hal::Lang;

/// The catalog's closed list of usage categories, in menu order, with their labels.
const USES: [(&str, &str, &str); 14] = [
    (
        "desktop",
        "Everyday computer",
        "Ordinateur de tous les jours",
    ),
    ("beginner", "New to Linux", "Débuter sous Linux"),
    ("gaming", "Gaming", "Jeux"),
    (
        "light",
        "Older and modest PCs",
        "Vieux PC et petites configurations",
    ),
    ("creative", "Sound, image, video", "Son, image, vidéo"),
    ("advanced", "Advanced users", "Utilisateurs avancés"),
    ("server", "Servers", "Serveurs"),
    (
        "homelab",
        "NAS, virtualisation, network",
        "NAS, virtualisation, réseau",
    ),
    ("privacy", "Security and privacy", "Sécurité et vie privée"),
    ("rescue", "Repair and backup", "Dépannage et sauvegarde"),
    ("media", "Media centre", "Médiacentre"),
    (
        "boards",
        "Raspberry Pi and ARM boards",
        "Raspberry Pi et cartes ARM",
    ),
    ("windows", "Windows", "Windows"),
    ("other", "Other systems", "Autres systèmes"),
];

/// The known uses among those given, in menu order; a name a newer catalog adds is left out.
#[must_use]
pub fn ordered<'a>(given: impl Iterator<Item = &'a String> + Clone) -> Vec<String> {
    USES.iter()
        .filter(|(id, ..)| given.clone().any(|g| g == id))
        .map(|(id, ..)| (*id).to_owned())
        .collect()
}

#[must_use]
pub fn label(id: &str, lang: Lang) -> String {
    USES.iter()
        .find(|(known, ..)| *known == id)
        .map_or_else(|| id.to_owned(), |(_, en, fr)| lang.pick(en, fr).to_owned())
}

/// Back from what a menu shows to the catalog name.
#[must_use]
pub fn id_of(shown: &str, lang: Lang) -> Option<String> {
    USES.iter()
        .find(|(_, en, fr)| lang.pick(en, fr) == shown)
        .map(|(id, ..)| (*id).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordre_du_menu_et_noms_inconnus_ecartes() {
        let given = [
            "server".to_owned(),
            "inconnu".to_owned(),
            "desktop".to_owned(),
        ];
        assert_eq!(ordered(given.iter()), ["desktop", "server"]);
    }

    #[test]
    fn libelle_et_retour_au_nom() {
        assert_eq!(label("gaming", Lang::Fr), "Jeux");
        assert_eq!(label("gaming", Lang::En), "Gaming");
        assert_eq!(label("futur", Lang::Fr), "futur");
        assert_eq!(id_of("Jeux", Lang::Fr).as_deref(), Some("gaming"));
        assert_eq!(id_of("Jeux", Lang::En), None);
    }
}
