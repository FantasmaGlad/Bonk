use std::path::Path;
use std::sync::Mutex;

use bonk_catalog::icons;
use bonk_catalog::manifest::{self, CatalogReport, Family, FamilyEntry, TRUSTED_KEYS};
use bonk_ui::catalog::CatalogState;
use serde_json::{Value, json};

use crate::{screen, vault};

static FAMILIES: Mutex<Vec<Family>> = Mutex::new(Vec::new());

/// The catalog and its signature, for Kotlin, which downloads them through the system's HTTPS.
pub(crate) fn addresses() -> Value {
    json!({
        "catalog": format!("{}{}", manifest::DEFAULT_BASE, manifest::MANIFEST),
        "signature": format!("{}{}", manifest::DEFAULT_BASE, manifest::SIGNATURE),
    })
}

/// A file Kotlin downloaded, or why it could not: an empty `failure` with an empty body is still a failure.
pub(crate) fn received(bytes: Vec<u8>, failure: String) -> Result<Vec<u8>, String> {
    match (failure.is_empty(), bytes.is_empty()) {
        (false, _) => Err(failure),
        (true, true) => Err("empty answer".to_owned()),
        (true, false) => Ok(bytes),
    }
}

/// Checks the signature of the catalog Kotlin downloaded (system HTTPS) and hands it to the screen. A file not
/// received makes `manifest::load` fall back to the last verified copy.
pub(crate) fn load(
    dir: &str,
    json: Result<Vec<u8>, String>,
    signature: Result<Vec<u8>, String>,
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let fetch = move |address: &str| -> Result<Vec<u8>, String> {
        if address.ends_with(".sig") {
            signature.clone()
        } else {
            json.clone()
        }
    };
    let loaded = manifest::load(
        manifest::DEFAULT_BASE,
        std::path::Path::new(dir),
        now,
        &TRUSTED_KEYS,
        &fetch,
    )
    .map_err(|error| error.to_string())?;
    let mut catalog = loaded.catalog;
    vault::set_grub(
        manifest::DEFAULT_BASE,
        catalog.grub.take(),
        std::mem::take(&mut catalog.vault_families),
    );
    *screen::lock(&FAMILIES) = catalog.families.clone();
    let report = report(catalog, loaded.saved_copy, dir);
    let shelf = bonk_ui::catalog::shelf(&report);
    let mut model = screen::lock(&screen::MODEL);
    let _ = model.update(bonk_ui::model::Message::CatalogLoaded(Ok(shelf)));
    Ok(())
}

fn report(catalog: manifest::Catalog, saved_copy: bool, dir: &str) -> CatalogReport {
    let mut images = catalog.images;
    images.sort_by(|a, b| {
        manifest::version_numbers(&b.version).cmp(&manifest::version_numbers(&a.version))
    });
    CatalogReport {
        schema_version: catalog.schema_version,
        sequence: catalog.sequence,
        expires_unix: catalog.expires_unix,
        saved_copy,
        families: catalog
            .families
            .into_iter()
            .map(|family| FamilyEntry {
                icon_path: icons::cached(std::path::Path::new(dir), &family),
                id: family.id,
                name: family.name,
                category: family.category,
                uses: family.uses,
                not_affiliated_with: family.not_affiliated_with,
            })
            .collect(),
        images,
        taxonomy: catalog.taxonomy,
    }
}

/// Logos not yet in the cache, for Kotlin to download: Rust checks each against its signed hash.
pub(crate) fn missing_icons(dir: &str) -> Value {
    let dir = Path::new(dir);
    let wanted: Vec<Value> = screen::lock(&FAMILIES)
        .iter()
        .filter(|family| icons::cached(dir, family).is_none())
        .map(
            |family| json!({"url": format!("{}{}", manifest::DEFAULT_BASE, family.icon), "sha256": family.icon_sha256}),
        )
        .collect();
    Value::Array(wanted)
}

pub(crate) fn store_icon(dir: &str, sha256: &str, bytes: &[u8]) -> Result<(), String> {
    let family = screen::lock(&FAMILIES)
        .iter()
        .find(|family| family.icon_sha256.eq_ignore_ascii_case(sha256))
        .cloned()
        .ok_or("no family has this icon")?;
    let path = icons::store(Path::new(dir), &family, bytes)?;
    show(&family.name, &path);
    Ok(())
}

fn show(name: &str, path: &Path) {
    let mut model = screen::lock(&screen::MODEL);
    if let CatalogState::Ready(systems) = &mut model.catalog {
        for system in systems.iter_mut().filter(|system| system.name == name) {
            system.icon = Some(path.to_path_buf());
        }
    }
}

#[cfg(test)]
mod tests {
    use bonk_ui::catalog::System;

    use super::*;

    const LOGO_SHA256: &str = "8f8cbb7dcf46e0bc7d53265749a6c17d116093a6ba95e442764060c76fd4a86c";

    #[test]
    fn un_fichier_non_recu_garde_sa_cause() {
        let cas = [
            (b"x".to_vec(), "", Ok(b"x".to_vec())),
            (
                Vec::new(),
                "HTTP 404: catalog.json",
                Err("HTTP 404: catalog.json".to_owned()),
            ),
            (Vec::new(), "", Err("empty answer".to_owned())),
        ];
        for (octets, cause, attendu) in cas {
            assert_eq!(received(octets, cause.to_owned()), attendu, "{cause}");
        }
    }

    #[test]
    fn sans_copie_verifiee_l_erreur_donne_la_cause_et_non_le_reseau() {
        let dir = std::env::temp_dir().join(format!("bonk-android-cause-{}", std::process::id()));
        let erreur = load(
            &dir.to_string_lossy(),
            Err("HTTP 404: catalog.json".to_owned()),
            Err("HTTP 404: catalog.json.sig".to_owned()),
        )
        .err()
        .unwrap_or_default();
        assert!(erreur.contains("HTTP 404: catalog.json"), "{erreur}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The only test that touches the shared model and family list.
    #[test]
    fn un_logo_manquant_est_demande_puis_garde_s_il_est_signe() -> Result<(), String> {
        let dir = std::env::temp_dir().join(format!("bonk-android-icons-{}", std::process::id()));
        let logo = b"png";
        let family: Family = serde_json::from_value(json!({
            "id": "debian", "name": "Debian", "icon": "icons/debian.png", "icon_sha256": LOGO_SHA256,
        }))
        .map_err(|e| e.to_string())?;
        *screen::lock(&FAMILIES) = vec![family];
        screen::lock(&screen::MODEL).catalog = CatalogState::Ready(vec![System {
            id: String::new(),
            name: "Debian".to_owned(),
            uses: Vec::new(),
            base: None,
            icon: None,
            choices: Vec::new(),
            not_affiliated_with: None,
        }]);
        let dir_text = dir.to_string_lossy().into_owned();
        let missing = missing_icons(&dir_text);
        assert_eq!(
            missing[0]["url"],
            format!("{}icons/debian.png", manifest::DEFAULT_BASE)
        );
        assert!(store_icon(&dir_text, LOGO_SHA256, b"autre").is_err());
        store_icon(&dir_text, LOGO_SHA256, logo)?;
        assert_eq!(missing_icons(&dir_text), json!([]));
        let shown = match &screen::lock(&screen::MODEL).catalog {
            CatalogState::Ready(systems) => systems[0].icon.clone(),
            _ => None,
        };
        assert!(shown.is_some_and(|path| path.starts_with(&dir)));
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
