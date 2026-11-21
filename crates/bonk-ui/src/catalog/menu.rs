use bonk_catalog::search;
use bonk_hal::Lang;

use super::{Choice, ChoiceSource, System};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MenuFilter {
    pub category: Option<String>,
    pub base: Option<String>,
    pub system: Option<String>,
    pub search: String,
    pub arch: Option<String>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MenuLists {
    pub categories: Vec<String>,
    pub bases: Vec<String>,
    pub systems: Vec<String>,
    /// Processors of the catalog, the most common first.
    pub arches: Vec<String>,
}

impl MenuFilter {
    fn query(&self) -> search::Query {
        search::Query {
            text: self.search.clone(),
            uses: self.category.clone(),
            base: self.base.clone(),
            arch: self.arch.clone(),
            ..search::Query::default()
        }
    }

    pub fn apply<'a>(&self, systems: &'a [System]) -> Vec<&'a System> {
        let query = self.query();
        systems
            .iter()
            .filter(|s| self.system.as_ref().is_none_or(|n| &s.name == n))
            .filter(|s| query.keeps_family(s.facts()))
            .filter(|s| !self.narrows_images() || !self.choices(s).is_empty())
            .collect()
    }

    fn narrows_images(&self) -> bool {
        self.arch.is_some() || !self.search.trim().is_empty()
    }

    pub fn choices<'a>(&self, system: &'a System) -> Vec<&'a Choice> {
        let query = self.query();
        system
            .choices
            .iter()
            .filter(|c| query.keeps_image(system.facts(), c.facts()))
            .collect()
    }

    /// In menu order: the keyboard index and the app's touched row refer to it.
    pub fn writable<'a>(&self, systems: &'a [System]) -> Vec<&'a Choice> {
        self.apply(systems)
            .into_iter()
            .flat_map(|system| self.choices(system))
            .filter(|choice| matches!(choice.source, ChoiceSource::Download { .. }))
            .collect()
    }

    #[must_use]
    pub fn lists(&self, systems: &[System], terms: &crate::terms::Terms) -> MenuLists {
        let narrowed = MenuFilter {
            system: None,
            ..self.clone()
        };
        let mut names: Vec<String> = narrowed
            .apply(systems)
            .iter()
            .map(|s| s.name.clone())
            .collect();
        names.sort();
        names.dedup();
        let mut arches: Vec<String> = systems
            .iter()
            .flat_map(|s| s.choices.iter().map(|c| c.arch.clone()))
            .collect();
        arches.sort_by_key(|arch| (arch_rank(arch), arch.clone()));
        arches.dedup();
        MenuLists {
            arches,
            categories: terms.uses().ordered(systems.iter().flat_map(|s| &s.uses)),
            bases: terms
                .bases()
                .ordered(systems.iter().filter_map(|s| s.base.as_ref())),
            systems: names,
        }
    }
}

fn arch_rank(arch: &str) -> u8 {
    match arch {
        "x86_64" => 0,
        "aarch64" => 1,
        _ => 2,
    }
}

#[must_use]
pub fn arch_label(arch: &str, lang: Lang) -> String {
    match arch {
        "x86_64" => "x86_64 (Intel, AMD)".to_owned(),
        "aarch64" => lang
            .pick("aarch64 (ARM, 64-bit)", "aarch64 (ARM, 64 bits)")
            .to_owned(),
        "i686" => lang
            .pick("i686 (Intel, AMD, 32-bit)", "i686 (Intel, AMD, 32 bits)")
            .to_owned(),
        "riscv64" => "riscv64 (RISC-V)".to_owned(),
        other => other.to_owned(),
    }
}

#[must_use]
pub fn all_arches(lang: Lang) -> &'static str {
    lang.pick("All processors", "Tous les processeurs")
}

#[must_use]
pub fn all_categories(lang: Lang) -> &'static str {
    lang.pick("All categories", "Toutes les catégories")
}

#[must_use]
pub fn all_bases(lang: Lang) -> &'static str {
    lang.pick("All bases", "Toutes les bases")
}

#[must_use]
pub fn all_systems(lang: Lang) -> &'static str {
    lang.pick("All systems", "Tous les systèmes")
}
