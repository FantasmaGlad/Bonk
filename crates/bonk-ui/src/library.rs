use std::path::PathBuf;

use crate::catalog::{CatalogState, find_sha256};
use crate::model::{Effect, Effects, Focus, Model, WriteState};
use crate::settings::Screen;
use crate::source::ImageSource;

/// An image kept in the download cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kept {
    pub name: String,
    /// The official hash it was downloaded for; checked again before each write.
    pub sha256: String,
    pub bytes: u64,
    /// An interrupted download: resumed by the next write of the same image, never written as is.
    pub partial: bool,
    /// Seconds since 1970.
    pub modified: Option<u64>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Listing {
    #[default]
    Loading,
    Ready(Vec<Kept>),
    Failed(String),
}

#[derive(Debug, Default)]
pub struct LibraryState {
    pub listing: Listing,
    /// The hash being removed: one removal at a time.
    pub removing: Option<String>,
    /// Why the last removal failed.
    pub error: Option<String>,
    /// Chosen to write before a drive was: fills the next write panel.
    pub chosen: Option<ImageSource>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryMessage {
    Open,
    Close,
    Listed(Result<Vec<Kept>, String>),
    Remove(String),
    Removed(Result<(), String>),
    /// Write the kept image with this hash.
    Write(String),
}

/// Reads `bonk cache list --json`; newest first.
pub fn parse(json: &[u8]) -> Result<Vec<Kept>, String> {
    let report: serde_json::Value =
        serde_json::from_slice(json).map_err(|error| error.to_string())?;
    let directory = report["directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or("no cache directory in the answer")?;
    let entries = report["entries"]
        .as_array()
        .ok_or("no image list in the answer")?;
    let mut kept: Vec<Kept> = entries
        .iter()
        .filter_map(|entry| {
            let name = entry["name"].as_str()?.to_owned();
            let sha256 = entry["sha256"].as_str()?.to_owned();
            Some(Kept {
                path: directory.join("images").join(&sha256).join(&name),
                bytes: entry["bytes"].as_u64()?,
                partial: entry["partial"].as_bool()?,
                modified: entry["modified"].as_u64(),
                name,
                sha256,
            })
        })
        .collect();
    kept.sort_by(|a, b| {
        b.modified
            .cmp(&a.modified)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(kept)
}

#[must_use]
pub fn list_args() -> Vec<String> {
    ["cache", "list", "--json"].map(str::to_owned).to_vec()
}

#[must_use]
pub fn remove_args(sha256: &str) -> Vec<String> {
    ["cache", "clean", sha256, "--json"]
        .map(str::to_owned)
        .to_vec()
}

impl Model {
    /// The kept images, once listed.
    #[must_use]
    pub fn kept(&self) -> &[Kept] {
        match &self.library.listing {
            Listing::Ready(kept) => kept,
            Listing::Loading | Listing::Failed(_) => &[],
        }
    }

    /// The catalog name of a kept image, when the catalog is read and still lists it.
    #[must_use]
    pub fn kept_label(&self, sha256: &str) -> Option<&str> {
        match &self.catalog {
            CatalogState::Ready(systems) => {
                find_sha256(systems, sha256).map(|choice| choice.label.as_str())
            }
            _ => None,
        }
    }

    /// A running write may be reading a kept file: nothing is removed meanwhile.
    #[must_use]
    pub fn can_remove_kept(&self) -> bool {
        self.library.removing.is_none() && !matches!(self.write, WriteState::Running { .. })
    }

    pub fn apply_library(&mut self, message: LibraryMessage) -> Effects {
        match message {
            LibraryMessage::Open => self.open_library(),
            LibraryMessage::Close => {
                self.screen = Screen::Main;
                self.focus = Some(Focus::Library);
                Vec::new()
            }
            LibraryMessage::Listed(result) => {
                self.library.listing = match result {
                    Ok(kept) => Listing::Ready(kept),
                    Err(error) => Listing::Failed(error),
                };
                self.fix_focus();
                Vec::new()
            }
            LibraryMessage::Remove(sha256) => {
                if !self.can_remove_kept() {
                    return Vec::new();
                }
                self.library.error = None;
                self.library.removing = Some(sha256.clone());
                vec![Effect::RemoveKept(sha256)]
            }
            LibraryMessage::Removed(result) => {
                self.library.removing = None;
                self.library.error = result.err();
                vec![Effect::ListKept]
            }
            LibraryMessage::Write(sha256) => self.write_kept(&sha256),
        }
    }

    fn open_library(&mut self) -> Effects {
        self.screen = Screen::Library;
        self.focus = Some(Focus::CloseLibrary);
        self.library.listing = Listing::Loading;
        self.library.error = None;
        let mut effects = vec![Effect::ListKept];
        if matches!(
            self.catalog,
            CatalogState::NotLoaded | CatalogState::Failed(_)
        ) {
            self.catalog = CatalogState::Loading;
            effects.push(Effect::LoadCatalog);
        }
        effects
    }

    /// Back to the drives: the open write panel takes the image now, else the next one that opens.
    fn write_kept(&mut self, sha256: &str) -> Effects {
        let Some(kept) = self
            .kept()
            .iter()
            .find(|kept| kept.sha256 == sha256 && !kept.partial)
        else {
            return Vec::new();
        };
        let source = ImageSource::Kept {
            path: kept.path.clone(),
            sha256: kept.sha256.clone(),
        };
        self.screen = Screen::Main;
        self.requested = None;
        if matches!(self.write, WriteState::Draft { .. }) {
            self.fill_draft(&source);
            self.focus = Some(Focus::Action);
        } else {
            self.library.chosen = Some(source);
            self.focus = None;
        }
        Vec::new()
    }

    /// Tab order of the library page.
    #[must_use]
    pub fn library_focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::CloseLibrary];
        for kept in self.kept() {
            if !kept.partial {
                order.push(Focus::KeptWrite(kept.sha256.clone()));
            }
            order.push(Focus::KeptRemove(kept.sha256.clone()));
        }
        order
    }
}

/// `2026-11-12` from seconds since 1970, in UTC.
#[must_use]
pub fn day(seconds: u64) -> String {
    // Days to a civil date, from Howard Hinnant's algorithm; no date library for one line of text.
    let days = i64::try_from(seconds / 86_400).unwrap_or(0) + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * of_year + 2) / 153;
    let day = of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    use super::*;
    use crate::model::Message;

    const LISTE: &str = r#"{"schema_version":1,"directory":"/cache","total_bytes":12,"removed_bytes":null,
        "entries":[
            {"name":"ancienne.iso","sha256":"aa","bytes":5,"partial":false,"modified":100},
            {"name":"recente.iso","sha256":"bb","bytes":4,"partial":false,"modified":200},
            {"name":"coupee.iso.part","sha256":"cc","bytes":3,"partial":true}
        ]}"#;

    fn ouverte() -> Model {
        let mut model = Model::default();
        model.apply(Message::Library(LibraryMessage::Open));
        model.apply(Message::Library(LibraryMessage::Listed(parse(
            LISTE.as_bytes(),
        ))));
        model
    }

    #[test]
    fn la_liste_se_lit_la_plus_recente_en_tete() {
        let kept = parse(LISTE.as_bytes()).expect("liste comprise");
        let names: Vec<_> = kept.iter().map(|k| k.name.as_str()).collect();
        assert_eq!(names, ["recente.iso", "ancienne.iso", "coupee.iso.part"]);
        assert_eq!(kept[0].path, PathBuf::from("/cache/images/bb/recente.iso"));
        assert_eq!(kept[2].modified, None);
        assert!(parse(b"{}").is_err());
    }

    #[test]
    fn ouvrir_lit_le_cache_et_le_catalogue() {
        let mut model = Model::default();
        let effects = model.apply(Message::Library(LibraryMessage::Open));
        assert_eq!(model.screen, Screen::Library);
        assert!(matches!(
            effects[..],
            [Effect::ListKept, Effect::LoadCatalog]
        ));
        model.navigate(crate::model::Nav::Escape);
        assert_eq!(model.screen, Screen::Main);
        assert_eq!(model.focus, Some(Focus::Library));
    }

    #[test]
    fn un_telechargement_coupe_ne_s_ecrit_pas() {
        let mut model = ouverte();
        assert!(
            !model
                .library_focus_order()
                .contains(&Focus::KeptWrite("cc".into()))
        );
        model.apply(Message::Library(LibraryMessage::Write("cc".into())));
        assert_eq!(model.screen, Screen::Library);
        assert_eq!(model.library.chosen, None);
    }

    #[test]
    fn ecrire_sans_cle_remplit_le_prochain_panneau() {
        let mut model = ouverte();
        model.apply(Message::Library(LibraryMessage::Write("bb".into())));
        assert_eq!(model.screen, Screen::Main);
        assert_eq!(
            model.library.chosen,
            Some(ImageSource::Kept {
                path: PathBuf::from("/cache/images/bb/recente.iso"),
                sha256: "bb".into()
            })
        );
    }

    #[test]
    fn une_suppression_a_la_fois_puis_relecture() {
        let mut model = ouverte();
        let effects = model.apply(Message::Library(LibraryMessage::Remove("aa".into())));
        assert_eq!(effects, [Effect::RemoveKept("aa".into())]);
        assert!(
            model
                .apply(Message::Library(LibraryMessage::Remove("bb".into())))
                .is_empty()
        );
        let effects = model.apply(Message::Library(LibraryMessage::Removed(Err(
            "refusé".into()
        ))));
        assert_eq!(effects, [Effect::ListKept]);
        assert_eq!(model.library.error.as_deref(), Some("refusé"));
        assert!(model.can_remove_kept());
    }

    #[test]
    fn dates() {
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(951_782_400), "2000-02-29");
        assert_eq!(day(1_794_441_600), "2026-11-12");
        assert_eq!(day(1_794_527_999), "2026-11-12");
    }
}
