use crate::catalog::{Choice, System};
use crate::terms::Terms;

use super::labels::{Order, ordered};

/// What the catalog tab narrows to. Empty: every system.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BrowseFilter {
    pub search: String,
    pub uses: Option<String>,
    pub base: Option<String>,
    pub desktop: Option<String>,
    pub media: Option<String>,
}

/// The values each filter offers, among the systems of the catalog.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Options {
    pub uses: Vec<String>,
    pub bases: Vec<String>,
    pub desktops: Vec<String>,
    pub media: Vec<String>,
}

impl BrowseFilter {
    pub fn apply<'a>(&self, systems: &'a [System]) -> Vec<&'a System> {
        systems.iter().filter(|system| self.keeps(system)).collect()
    }

    /// A variant of the system must match the desktop and the medium together.
    fn keeps(&self, system: &System) -> bool {
        self.uses.as_ref().is_none_or(|id| system.uses.contains(id))
            && (self.base.is_none() || system.base == self.base)
            && self.matches_search(system)
            && system
                .choices
                .iter()
                .any(|choice| self.keeps_choice(choice))
    }

    #[must_use]
    pub fn keeps_choice(&self, choice: &Choice) -> bool {
        same(self.desktop.as_ref(), choice.desktop.as_ref())
            && same(self.media.as_ref(), choice.media.as_ref())
    }

    /// Kubuntu is filed under Ubuntu: the variants' names count as much as the system's.
    fn matches_search(&self, system: &System) -> bool {
        let wanted = self.search.trim().to_lowercase();
        wanted.is_empty()
            || system.name.to_lowercase().contains(&wanted)
            || system
                .choices
                .iter()
                .any(|choice| choice.name.to_lowercase().contains(&wanted))
    }
}

fn same(wanted: Option<&String>, value: Option<&String>) -> bool {
    wanted.is_none_or(|wanted| value == Some(wanted))
}

#[must_use]
pub fn options(systems: &[System], terms: &Terms) -> Options {
    let choices = || systems.iter().flat_map(|system| &system.choices);
    Options {
        uses: terms.uses().ordered(systems.iter().flat_map(|s| &s.uses)),
        bases: terms
            .bases()
            .ordered(systems.iter().filter_map(|s| s.base.as_ref())),
        desktops: ordered(
            choices().filter_map(|c| c.desktop.as_deref()),
            Order::Desktops,
        ),
        media: ordered(choices().filter_map(|c| c.media.as_deref()), Order::Media),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ChoiceSource;

    fn choice(name: &str, desktop: &str, media: &str) -> Choice {
        Choice {
            id: format!("{name}-{desktop}-{media}"),
            name: name.to_owned(),
            label: "1".to_owned(),
            end_of_life: false,
            note: None,
            source: ChoiceSource::NotForUsb,
            desktop: Some(desktop.to_owned()),
            media: Some(media.to_owned()),
        }
    }

    fn system(name: &str, uses: &[&str], choices: Vec<Choice>) -> System {
        System {
            name: name.to_owned(),
            uses: uses.iter().map(|u| (*u).to_owned()).collect(),
            base: Some("debian".to_owned()),
            icon: None,
            choices,
        }
    }

    fn shelf() -> Vec<System> {
        vec![
            system(
                "Ubuntu",
                &["office"],
                vec![
                    choice("Ubuntu", "gnome", "usb"),
                    choice("Kubuntu", "kde", "usb"),
                ],
            ),
            system(
                "Raspberry Pi OS",
                &["arm"],
                vec![choice("Raspberry Pi OS", "pixel", "sd")],
            ),
        ]
    }

    fn names(filter: &BrowseFilter, systems: &[System]) -> Vec<String> {
        filter
            .apply(systems)
            .iter()
            .map(|s| s.name.clone())
            .collect()
    }

    #[test]
    fn la_recherche_trouve_aussi_une_variante_sous_son_systeme() {
        let systems = shelf();
        let filter = BrowseFilter {
            search: " kubu ".to_owned(),
            ..BrowseFilter::default()
        };
        assert_eq!(names(&filter, &systems), ["Ubuntu"]);
    }

    #[test]
    fn bureau_et_support_doivent_aller_ensemble_sur_une_variante() {
        let systems = shelf();
        let kde_sd = BrowseFilter {
            desktop: Some("kde".to_owned()),
            media: Some("sd".to_owned()),
            ..BrowseFilter::default()
        };
        assert!(names(&kde_sd, &systems).is_empty());
        let sd = BrowseFilter {
            media: Some("sd".to_owned()),
            ..BrowseFilter::default()
        };
        assert_eq!(names(&sd, &systems), ["Raspberry Pi OS"]);
    }

    #[test]
    fn les_options_suivent_l_ordre_connu_puis_l_inconnu() {
        let mut systems = shelf();
        systems[0].choices.push(choice("Ubuntu", "neuf", "usb"));
        let offered = options(&systems, &Terms::default());
        assert_eq!(offered.desktops, ["gnome", "kde", "pixel", "neuf"]);
        assert_eq!(offered.media, ["usb", "sd"]);
    }
}
