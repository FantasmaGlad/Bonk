use crate::manifest::{Family, FamilyEntry, Image};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub uses: Option<String>,
    pub base: Option<String>,
    pub desktop: Option<String>,
    pub media: Option<String>,
    pub arch: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct FamilyFacts<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub uses: &'a [String],
    pub base: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct ImageFacts<'a> {
    pub name: &'a str,
    pub edition: Option<&'a str>,
    pub desktop: Option<&'a str>,
    pub media: &'a str,
    pub arch: &'a str,
}

impl<'a> FamilyFacts<'a> {
    #[must_use]
    pub fn of(family: &'a Family) -> Self {
        Self {
            id: &family.id,
            name: &family.name,
            uses: &family.uses,
            base: family.category.as_deref(),
        }
    }

    #[must_use]
    pub fn of_entry(family: &'a FamilyEntry) -> Self {
        Self {
            id: &family.id,
            name: &family.name,
            uses: &family.uses,
            base: family.category.as_deref(),
        }
    }
}

impl<'a> ImageFacts<'a> {
    #[must_use]
    pub fn of(image: &'a Image) -> Self {
        Self {
            name: &image.name,
            edition: image.edition.as_deref(),
            desktop: image.desktop.as_deref(),
            media: media_of(image.media.as_deref(), image.usb_boot),
            arch: &image.arch,
        }
    }
}

impl Query {
    #[must_use]
    pub fn keeps_family(&self, family: FamilyFacts<'_>) -> bool {
        self.uses
            .as_ref()
            .is_none_or(|wanted| family.uses.contains(wanted))
            && self
                .base
                .as_deref()
                .is_none_or(|wanted| family.base == Some(wanted))
    }

    #[must_use]
    pub fn keeps_image(&self, family: FamilyFacts<'_>, image: ImageFacts<'_>) -> bool {
        self.keeps_kind(image) && self.names(family, image)
    }

    /// Desktop, medium and processor must hold on one image together.
    #[must_use]
    pub fn keeps_kind(&self, image: ImageFacts<'_>) -> bool {
        same(self.desktop.as_deref(), image.desktop)
            && same(self.media.as_deref(), Some(image.media))
            && same(self.arch.as_deref(), Some(image.arch))
    }

    fn names(&self, family: FamilyFacts<'_>, image: ImageFacts<'_>) -> bool {
        let haystack = fold(
            &[
                family.id,
                family.name,
                image.name,
                image.edition.unwrap_or(""),
            ]
            .join(" "),
        );
        fold(&self.text)
            .split_whitespace()
            .all(|word| haystack.contains(word))
    }
}

fn same(wanted: Option<&str>, value: Option<&str>) -> bool {
    wanted.is_none_or(|wanted| value == Some(wanted))
}

/// What the file is written to; a catalog without the field says so through `usb_boot`.
#[must_use]
pub fn media_of(declared: Option<&str>, usb_boot: bool) -> &str {
    match declared {
        Some(media) => media,
        None if usb_boot => "usb",
        None => "optical",
    }
}

/// Lower case without the accents that Unicode decomposes, as `normalize("NFD")` then dropping the marks.
#[must_use]
pub fn fold(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .map(base_letter)
        .collect()
}

fn base_letter(c: char) -> char {
    const FOLDS: [(&str, char); 19] = [
        ("àáâãäåāăą", 'a'),
        ("çćĉċč", 'c'),
        ("ď", 'd'),
        ("èéêëēĕėęě", 'e'),
        ("ĝğġģ", 'g'),
        ("ĥ", 'h'),
        ("ìíîïĩīĭį", 'i'),
        ("ĵ", 'j'),
        ("ķ", 'k'),
        ("ĺļľ", 'l'),
        ("ñńņň", 'n'),
        ("òóôõöōŏő", 'o'),
        ("ŕŗř", 'r'),
        ("śŝşš", 's'),
        ("ţť", 't'),
        ("ùúûüũūŭůűų", 'u'),
        ("ŵ", 'w'),
        ("ýÿŷ", 'y'),
        ("źżž", 'z'),
    ];
    FOLDS
        .iter()
        .find(|(accented, _)| accented.contains(c))
        .map_or(c, |&(_, base)| base)
}

#[cfg(test)]
mod tests;
