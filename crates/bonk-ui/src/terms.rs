use bonk_catalog::manifest::{Taxonomy, Term};
use bonk_hal::Lang;

/// Used when the catalog carries no taxonomy: one published before it, or a saved copy of one.
const USES: [(&str, &str, &str); 18] = [
    ("office", "Office and everyday", "Bureautique"),
    ("beginner", "Beginners", "Débutant"),
    ("gaming", "Gaming", "Jeux"),
    ("creative", "Sound, image, video", "Création"),
    ("light", "Older PCs", "Vieux PC"),
    ("education", "Education", "Éducation"),
    ("security", "Cybersecurity", "Cybersécurité"),
    ("privacy", "Privacy", "Vie privée"),
    ("immutable", "Immutable systems", "Systèmes immuables"),
    ("expert", "Experts", "Experts"),
    ("server", "Servers", "Serveurs"),
    ("homelab", "NAS and virtualisation", "NAS et virtualisation"),
    ("network", "Network and firewall", "Réseau et pare-feu"),
    ("rescue", "Repair and backup", "Dépannage et sauvegarde"),
    ("media", "Media centre", "Médiacentre"),
    (
        "boards",
        "Raspberry Pi and ARM boards",
        "Raspberry Pi et cartes ARM",
    ),
    ("exotic", "Exotic", "Exotique"),
    ("windows", "Windows", "Windows"),
];

const BASES: [(&str, &str, &str); 8] = [
    ("debian", "Debian", "Debian"),
    ("ubuntu", "Ubuntu", "Ubuntu"),
    ("arch", "Arch Linux", "Arch Linux"),
    ("fedora", "Fedora and Red Hat", "Fedora et Red Hat"),
    ("opensuse", "openSUSE", "openSUSE"),
    ("independent", "Independent", "Indépendant"),
    ("bsd", "BSD and Unix", "BSD et Unix"),
    ("windows", "Windows", "Windows"),
];

fn built_in(list: &[(&str, &str, &str)]) -> Vec<Term> {
    list.iter()
        .map(|(id, en, fr)| Term {
            id: (*id).to_owned(),
            en: (*en).to_owned(),
            fr: (*fr).to_owned(),
        })
        .collect()
}

/// The usage categories and technical bases with their labels, as the catalog publishes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terms {
    uses: Vec<Term>,
    bases: Vec<Term>,
}

impl Default for Terms {
    fn default() -> Self {
        Self::from_catalog(None)
    }
}

impl Terms {
    #[must_use]
    pub fn from_catalog(taxonomy: Option<&Taxonomy>) -> Self {
        let or_built_in = |given: Option<&Vec<Term>>, own: &[(&str, &str, &str)]| match given {
            Some(terms) if !terms.is_empty() => terms.clone(),
            _ => built_in(own),
        };
        Self {
            uses: or_built_in(taxonomy.map(|t| &t.uses), &USES),
            bases: or_built_in(taxonomy.map(|t| &t.bases), &BASES),
        }
    }

    #[must_use]
    pub fn uses(&self) -> List<'_> {
        List(&self.uses)
    }

    #[must_use]
    pub fn bases(&self) -> List<'_> {
        List(&self.bases)
    }
}

/// One closed list, in menu order.
#[derive(Debug, Clone, Copy)]
pub struct List<'a>(&'a [Term]);

impl List<'_> {
    /// The known names among those given, in menu order; a name the list lacks is left out.
    #[must_use]
    pub fn ordered<'b>(self, given: impl Iterator<Item = &'b String> + Clone) -> Vec<String> {
        self.0
            .iter()
            .filter(|term| given.clone().any(|g| *g == term.id))
            .map(|term| term.id.clone())
            .collect()
    }

    #[must_use]
    pub fn label(self, id: &str, lang: Lang) -> String {
        self.0
            .iter()
            .find(|term| term.id == id)
            .map_or_else(|| id.to_owned(), |term| text(term, lang).to_owned())
    }

    /// Back from what a menu shows to the catalog name.
    #[must_use]
    pub fn id_of(self, shown: &str, lang: Lang) -> Option<String> {
        self.0
            .iter()
            .find(|term| text(term, lang) == shown)
            .map(|term| term.id.clone())
    }
}

fn text(term: &Term, lang: Lang) -> &str {
    match lang {
        Lang::En => &term.en,
        Lang::Fr => &term.fr,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(id: &str, en: &str, fr: &str) -> Term {
        Term {
            id: id.to_owned(),
            en: en.to_owned(),
            fr: fr.to_owned(),
        }
    }

    #[test]
    fn ordre_du_menu_et_noms_inconnus_ecartes() {
        let terms = Terms::default();
        let given = [
            "server".to_owned(),
            "desktop".to_owned(),
            "office".to_owned(),
        ];
        assert_eq!(terms.uses().ordered(given.iter()), ["office", "server"]);
    }

    #[test]
    fn libelle_et_retour_au_nom() {
        let terms = Terms::default();
        assert_eq!(terms.uses().label("security", Lang::Fr), "Cybersécurité");
        assert_eq!(terms.bases().label("bsd", Lang::En), "BSD and Unix");
        assert_eq!(terms.uses().label("futur", Lang::Fr), "futur");
        assert_eq!(
            terms.bases().id_of("Indépendant", Lang::Fr).as_deref(),
            Some("independent")
        );
        assert_eq!(terms.bases().id_of("Indépendant", Lang::En), None);
    }

    #[test]
    fn la_liste_du_catalogue_remplace_la_copie_integree() {
        let taxonomy = Taxonomy {
            uses: vec![term("retro", "Retro gaming", "Rétro")],
            bases: Vec::new(),
        };
        let terms = Terms::from_catalog(Some(&taxonomy));
        assert_eq!(terms.uses().label("retro", Lang::Fr), "Rétro");
        assert_eq!(terms.uses().label("office", Lang::En), "office");
        assert_eq!(terms.bases().label("debian", Lang::En), "Debian");
    }
}
