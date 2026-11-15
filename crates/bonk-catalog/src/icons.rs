use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use ring::digest::{SHA256, digest};

use crate::manifest::{Family, hex, replace};

/// Named by the signed hash: a new logo never reuses an old file.
fn file(dir: &Path, family: &Family) -> Option<PathBuf> {
    let extension = Path::new(&family.icon).extension()?.to_str()?;
    Some(
        dir.join("icons")
            .join(format!("{}.{extension}", family.icon_sha256)),
    )
}

fn matches(family: &Family, bytes: &[u8]) -> bool {
    hex(digest(&SHA256, bytes).as_ref()).eq_ignore_ascii_case(&family.icon_sha256)
}

/// The cached logo, checked again by its signed hash at each read.
#[must_use]
pub fn cached(dir: &Path, family: &Family) -> Option<PathBuf> {
    let path = file(dir, family)?;
    fs::read(&path)
        .is_ok_and(|bytes| matches(family, &bytes))
        .then_some(path)
}

/// Kept only if the bytes match the signed hash.
///
/// # Errors
/// The bytes are not the signed logo, or the cache cannot be written.
pub fn store(dir: &Path, family: &Family, bytes: &[u8]) -> Result<PathBuf, String> {
    if !matches(family, bytes) {
        return Err(format!(
            "{}: icon does not match its signed hash",
            family.id
        ));
    }
    let path = file(dir, family).ok_or("icon without extension")?;
    path.parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| replace(&path, bytes))
        .map_err(|error| error.to_string())?;
    Ok(path)
}

/// Fetches a logo by its full address.
pub type Fetch<'a> = dyn Fn(&str) -> Result<Vec<u8>, String> + Sync + 'a;

/// Every family's logo, `workers` downloads at a time: one by one, a cold cache took 20 s for 96 logos. A
/// logo shared by two families is fetched once, so two threads never write the same file.
pub fn paths(
    base: &str,
    dir: &Path,
    families: &[Family],
    workers: usize,
    fetch: &Fetch<'_>,
) -> Vec<Option<PathBuf>> {
    let mut unique: BTreeMap<&str, &Family> = BTreeMap::new();
    for family in families {
        unique.entry(family.icon_sha256.as_str()).or_insert(family);
    }
    let unique: Vec<&Family> = unique.into_values().collect();
    let found = fetch_all(base, dir, &unique, workers, fetch);
    families
        .iter()
        .map(|family| found.get(family.icon_sha256.as_str()).cloned().flatten())
        .collect()
}

fn fetch_all(
    base: &str,
    dir: &Path,
    families: &[&Family],
    workers: usize,
    fetch: &Fetch<'_>,
) -> BTreeMap<String, Option<PathBuf>> {
    let next = AtomicUsize::new(0);
    let worker = || {
        let mut mine = Vec::new();
        while let Some(family) = families.get(next.fetch_add(1, Ordering::Relaxed)) {
            let path = crate::manifest::icon_path(base, dir, family, fetch);
            mine.push((family.icon_sha256.clone(), path));
        }
        mine
    };
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers.clamp(1, families.len().max(1)))
            .map(|_| scope.spawn(worker))
            .collect();
        // A worker that panicked leaves its logos absent: the list still shows the systems.
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn family(icon_sha256: String) -> Family {
        serde_json::from_value(serde_json::json!({
            "id": "debian", "name": "Debian", "icon": "icons/debian.png", "icon_sha256": icon_sha256,
        }))
        .unwrap_or_else(|error| panic!("famille d'essai : {error}"))
    }

    fn folder() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bonk-icons-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn les_logos_arrivent_en_parallele_et_une_seule_fois_chacun() {
        use std::sync::atomic::AtomicUsize;
        use std::time::{Duration, Instant};

        let dir = folder().with_extension("parallele");
        let logo = |n: u8| vec![n; 4];
        let mut families: Vec<Family> = (0..8)
            .map(|n| Family {
                icon: format!("icons/{n}.png"),
                ..family(hex(digest(&SHA256, &logo(n)).as_ref()))
            })
            .collect();
        families.push(families[0].clone());
        let calls = AtomicUsize::new(0);
        let fetch = |address: &str| -> Result<Vec<u8>, String> {
            calls.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(100));
            let n: u8 = address
                .trim_start_matches("https://c/icons/")
                .trim_end_matches(".png")
                .parse()
                .map_err(|_| format!("adresse inattendue : {address}"))?;
            Ok(logo(n))
        };
        let started = Instant::now();
        let found = paths("https://c/", &dir, &families, 4, &fetch);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(calls.load(Ordering::Relaxed), 8);
        assert!(found.iter().all(Option::is_some));
        assert_eq!(found[0], found[8]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn seul_le_logo_signe_est_garde_puis_relu() -> Result<(), String> {
        let dir = folder();
        let logo = b"png";
        let signed = family(hex(digest(&SHA256, logo).as_ref()));
        assert!(store(&dir, &signed, b"autre").is_err());
        assert_eq!(cached(&dir, &signed), None);
        let path = store(&dir, &signed, logo)?;
        assert_eq!(cached(&dir, &signed), Some(path.clone()));
        fs::write(&path, b"remplace").map_err(|e| e.to_string())?;
        assert_eq!(
            cached(&dir, &signed),
            None,
            "un logo modifié n'est plus servi"
        );
        let _ = fs::remove_dir_all(&dir);
        Ok(())
    }
}
