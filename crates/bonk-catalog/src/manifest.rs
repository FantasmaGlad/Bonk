use crate::sums::is_sha256;
use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ring::digest::{SHA256, digest};
use ring::signature::{ED25519, UnparsedPublicKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// The current key, in the publishing secrets, and an offline backup that keeps publishing possible without a
/// new Bonk release if the first is lost.
pub const TRUSTED_KEYS: [[u8; 32]; 2] = [
    [
        0x0a, 0xa9, 0xf4, 0x66, 0x76, 0x3f, 0x33, 0x45, 0x0a, 0xe8, 0x21, 0x17, 0xf3, 0x00, 0xd4,
        0x90, 0x32, 0x62, 0xa8, 0x4a, 0x21, 0x11, 0xae, 0x31, 0x3d, 0xf1, 0xe8, 0xb1, 0x5a, 0xb1,
        0x67, 0xd4,
    ],
    [
        0xed, 0xce, 0x26, 0x90, 0xbc, 0xab, 0xa1, 0x8c, 0x90, 0xfd, 0x01, 0x0a, 0x80, 0x87, 0xab,
        0x3a, 0x9d, 0x78, 0x85, 0x5d, 0xae, 0x4c, 0xb0, 0xe6, 0xb3, 0x36, 0xf7, 0x17, 0x30, 0xc1,
        0xe0, 0xee,
    ],
];

/// A system as the choice screen shows it: its versions are its images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Family {
    /// Stable identifier: `ubuntu`.
    pub id: String,
    pub name: String,
    /// Group shown to sort systems: `Debian derivatives`. Absent from older catalogs.
    #[serde(default)]
    pub category: Option<String>,
    /// Logo, relative to the manifest. Its hash, covered by the signature, keeps it from being replaced.
    pub icon: String,
    pub icon_sha256: String,
}

/// Life cycle of a version: what marks an old version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// Long-term maintained by the publisher (Ubuntu 26.04 LTS).
    Lts,
    /// Current version.
    Stable,
    /// Still offered, no longer maintained by the publisher: the interface says so.
    EndOfLife,
}

/// An offered image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Image {
    /// Stable identifier: `debian-13-netinst-x86_64`.
    pub id: String,
    /// Identifier of the [`Family`].
    pub family: String,
    pub name: String,
    pub version: String,
    pub channel: Channel,
    /// Release date `YYYY-MM-DD`, to sort versions.
    pub released: String,
    #[serde(default)]
    pub edition: Option<String>,
    pub arch: String,
    pub homepage: String,
    /// What to know before choosing it: 90-day evaluation, CD only.
    #[serde(default)]
    pub note: Option<String>,
    /// The file boots once written to a USB drive. False: CD or virtual machine only.
    #[serde(default = "boots_from_usb")]
    pub usb_boot: bool,
    /// Other addresses of the same file: same hash, same size. `.onion` ones may be plain http.
    #[serde(default)]
    pub mirrors: Vec<String>,
    #[serde(flatten)]
    pub source: ImageSource,
}

fn boots_from_usb() -> bool {
    true
}

/// Where to get the image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ImageSource {
    /// Official link, hash read from the publisher.
    Download {
        url: String,
        sha256: String,
        size_bytes: u64,
    },
    /// Publisher without a stable link (Windows): the user downloads it.
    OfficialPage { official_page: String },
}

/// `24.04.5` sorts after `26.04.1` even if released later, and `9` before `10`.
#[must_use]
pub fn version_numbers(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|part| part.parse().ok())
        .collect()
}

/// A system in the report of `bonk catalog list --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FamilyEntry {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
    /// Logo checked by its hash, in the local cache; absent if it could not be read.
    #[serde(default)]
    pub icon_path: Option<std::path::PathBuf>,
}

/// Report of `bonk catalog list --json`: what interfaces read, one type for both sides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CatalogReport {
    pub schema_version: u32,
    pub sequence: u64,
    pub expires_unix: u64,
    /// Saved copy, because the network was missing.
    pub saved_copy: bool,
    pub families: Vec<FamilyEntry>,
    pub images: Vec<Image>,
}

/// The manifest, once verified.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    pub schema_version: u32,
    /// Grows at each publication: a mirror cannot serve an older manifest to someone who saw a newer one.
    pub sequence: u64,
    pub expires_unix: u64,
    pub families: Vec<Family>,
    pub images: Vec<Image>,
    /// Boot loader of the vault. Absent from older catalogs.
    #[serde(default)]
    pub grub: Option<Grub>,
}

/// A file published next to the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Asset {
    /// Relative to the manifest.
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

/// GRUB built from the signed GNU archive, with its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Grub {
    pub version: String,
    pub released: String,
    /// Changes when the build changes, even for the same version.
    pub build: String,
    /// One standalone EFI image per architecture: `x86_64`, later `aarch64` and `riscv64`.
    pub efi: BTreeMap<String, Asset>,
    pub source: Asset,
    pub source_signature: Asset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    BadSignature,
    Unreadable(String),
    UnsupportedVersion(u32),
    Expired {
        expires_unix: u64,
    },
    Rollback {
        sequence: u64,
        seen: u64,
    },
    InvalidImage {
        id: String,
        reason: &'static str,
    },
    InvalidGrub(&'static str),
    /// Neither the network nor a saved copy.
    Unavailable(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSignature => f.write_str("catalog signature does not match"),
            Self::Unreadable(detail) => write!(f, "unreadable catalog: {detail}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "catalog format {version} needs a newer version of Bonk")
            }
            Self::Expired { expires_unix } => {
                write!(f, "catalog expired (at unix time {expires_unix})")
            }
            Self::Rollback { sequence, seen } => {
                write!(
                    f,
                    "catalog {sequence} is older than the one already seen ({seen})"
                )
            }
            Self::InvalidImage { id, reason } => write!(f, "catalog image '{id}': {reason}"),
            Self::InvalidGrub(reason) => write!(f, "catalog GRUB entry: {reason}"),
            Self::Unavailable(detail) => write!(f, "catalog unavailable: {detail}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Allowed keys, the time, the highest sequence already seen.
#[derive(Debug, Clone, Copy)]
pub struct Trust<'a> {
    /// Ed25519 public keys: current and backup, to survive a lost key.
    pub keys: &'a [[u8; 32]],
    pub now_unix: u64,
    pub seen_sequence: u64,
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.' || b == b'_'
        })
}

/// `YYYY-MM-DD` shape only: enough for text order to be date order.
fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                *b == b'-'
            } else {
                b.is_ascii_digit()
            }
        })
}

/// Stays under the manifest directory: no absolute path, no parent, no other host.
fn is_relative(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && part != "..")
}

fn check_grub(grub: &Grub) -> Result<(), &'static str> {
    if grub.efi.is_empty() {
        return Err("no EFI image");
    }
    let assets = grub
        .efi
        .values()
        .chain([&grub.source, &grub.source_signature]);
    for asset in assets {
        if !is_relative(&asset.path) {
            return Err("file path must be relative");
        }
        if !is_sha256(&asset.sha256) {
            return Err("sha256 must be 64 hexadecimal characters");
        }
    }
    Ok(())
}

fn check_family(family: &Family) -> Result<(), &'static str> {
    if !is_identifier(&family.id) {
        return Err("identifier must be lowercase ASCII");
    }
    if !is_relative(&family.icon) {
        return Err("icon must be a relative path");
    }
    if !is_sha256(&family.icon_sha256) {
        return Err("icon sha256 must be 64 hexadecimal characters");
    }
    Ok(())
}

/// An onion service already encrypts the whole path: the only `http://` allowed.
fn is_mirror(address: &str) -> bool {
    let onion = address
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host.ends_with(".onion"));
    address.starts_with("https://") || onion
}

fn check_image(image: &Image) -> Result<(), &'static str> {
    if !is_identifier(&image.id) {
        return Err("identifier must be lowercase ASCII");
    }
    if !is_date(&image.released) {
        return Err("release date must be YYYY-MM-DD");
    }
    if !image.homepage.starts_with("https://") {
        return Err("homepage must be https");
    }
    if !image.mirrors.iter().all(|mirror| is_mirror(mirror)) {
        return Err("mirror must be https or an onion address");
    }
    match &image.source {
        ImageSource::Download { url, sha256, .. } => {
            if !url.starts_with("https://") {
                return Err("url must be https");
            }
            if !is_sha256(sha256) {
                return Err("sha256 must be 64 hexadecimal characters");
            }
        }
        ImageSource::OfficialPage { official_page } => {
            if !official_page.starts_with("https://") {
                return Err("official page must be https");
            }
        }
    }
    Ok(())
}

/// Checks the signature, and only then reads and validates the manifest.
///
/// # Errors
/// Bad signature, unreadable or newer format, manifest expired or older than the last seen, invalid image.
pub fn verify(
    manifest: &[u8],
    signature: &[u8],
    trust: &Trust<'_>,
) -> Result<Catalog, CatalogError> {
    let signed = trust.keys.iter().any(|key| {
        UnparsedPublicKey::new(&ED25519, key)
            .verify(manifest, signature)
            .is_ok()
    });
    if !signed {
        return Err(CatalogError::BadSignature);
    }
    // Version first: a newer format may not parse as this one.
    #[derive(Deserialize)]
    struct Header {
        schema_version: u32,
    }
    let unreadable = |e: serde_json::Error| CatalogError::Unreadable(e.to_string());
    let header: Header = serde_json::from_slice(manifest).map_err(unreadable)?;
    if header.schema_version != SCHEMA_VERSION {
        return Err(CatalogError::UnsupportedVersion(header.schema_version));
    }
    let catalog: Catalog = serde_json::from_slice(manifest).map_err(unreadable)?;
    if catalog.expires_unix <= trust.now_unix {
        return Err(CatalogError::Expired {
            expires_unix: catalog.expires_unix,
        });
    }
    if catalog.sequence < trust.seen_sequence {
        return Err(CatalogError::Rollback {
            sequence: catalog.sequence,
            seen: trust.seen_sequence,
        });
    }
    let mut families = HashSet::new();
    for family in &catalog.families {
        check_family(family).map_err(|reason| CatalogError::InvalidImage {
            id: family.id.clone(),
            reason,
        })?;
        if !families.insert(family.id.as_str()) {
            return Err(CatalogError::InvalidImage {
                id: family.id.clone(),
                reason: "duplicate identifier",
            });
        }
    }
    let mut ids = HashSet::new();
    for image in &catalog.images {
        if !families.contains(image.family.as_str()) {
            return Err(CatalogError::InvalidImage {
                id: image.id.clone(),
                reason: "unknown family",
            });
        }
        check_image(image).map_err(|reason| CatalogError::InvalidImage {
            id: image.id.clone(),
            reason,
        })?;
        if !ids.insert(image.id.as_str()) {
            return Err(CatalogError::InvalidImage {
                id: image.id.clone(),
                reason: "duplicate identifier",
            });
        }
    }
    if let Some(grub) = &catalog.grub {
        check_grub(grub).map_err(CatalogError::InvalidGrub)?;
    }
    Ok(catalog)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub catalog: Catalog,
    /// Saved copy, the network having failed. Still subject to the signature and the expiry.
    pub saved_copy: bool,
}

const MANIFEST: &str = "catalog.json";
const SIGNATURE: &str = "catalog.json.sig";
const SEEN: &str = "seen";

fn seen_sequence(dir: &Path) -> u64 {
    fs::read_to_string(dir.join(SEEN))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// Never leaves a half-written file: a cut keeps the old one.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let partial = path.with_extension("part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

/// Under `base` or, when only the network failed, the last verified copy. A file received but refused stays
/// an error: silently falling back would hide tampering.
///
/// # Errors
/// File refused by [`verify`], or neither network nor copy.
pub fn load(
    base: &str,
    dir: &Path,
    now_unix: u64,
    keys: &[[u8; 32]],
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<Loaded, CatalogError> {
    let trust = Trust {
        keys,
        now_unix,
        seen_sequence: seen_sequence(dir),
    };
    let received = fetch(&format!("{base}{MANIFEST}"))
        .and_then(|manifest| Ok((manifest, fetch(&format!("{base}{SIGNATURE}"))?)));
    match received {
        Ok((manifest, signature)) => {
            let catalog = verify(&manifest, &signature, &trust)?;
            // The cache is only a fallback: failing to write it does not stop reading the catalog.
            let saved = fs::create_dir_all(dir)
                .and_then(|()| replace(&dir.join(MANIFEST), &manifest))
                .and_then(|()| replace(&dir.join(SIGNATURE), &signature))
                .and_then(|()| replace(&dir.join(SEEN), catalog.sequence.to_string().as_bytes()));
            if let Err(error) = saved {
                log::warn!(target: "bonk::catalog", error = error.to_string().as_str(); "catalog copy not saved");
            }
            Ok(Loaded {
                catalog,
                saved_copy: false,
            })
        }
        Err(detail) => {
            let copy = fs::read(dir.join(MANIFEST))
                .and_then(|manifest| Ok((manifest, fs::read(dir.join(SIGNATURE))?)));
            let (manifest, signature) = copy.map_err(|_| CatalogError::Unavailable(detail))?;
            Ok(Loaded {
                catalog: verify(&manifest, &signature, &trust)?,
                saved_copy: true,
            })
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Fetches a file up to the given number of bytes.
pub type FetchSized<'a> = dyn Fn(&str, u64) -> Result<Vec<u8>, String> + 'a;

/// The bytes carry the signed hash of `asset`.
#[must_use]
pub fn asset_matches(asset: &Asset, bytes: &[u8]) -> bool {
    hex(digest(&SHA256, bytes).as_ref()).eq_ignore_ascii_case(&asset.sha256)
}

/// In the cache under `folder`, named by its signed hash, downloaded when missing, up to its signed size.
/// A cached copy is hashed again before use: a file changed on disk is downloaded anew.
///
/// # Errors
/// Download failure, or bytes that do not match the signed hash.
pub fn asset_path(
    base: &str,
    dir: &Path,
    folder: &str,
    asset: &Asset,
    fetch: &FetchSized<'_>,
) -> Result<PathBuf, String> {
    let name = Path::new(&asset.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let path = dir.join(folder).join(&asset.sha256).join(name);
    if fs::read(&path).is_ok_and(|bytes| asset_matches(asset, &bytes)) {
        return Ok(path);
    }
    let bytes = fetch(&format!("{base}{}", asset.path), asset.size_bytes)?;
    if !asset_matches(asset, &bytes) {
        return Err(format!("{} does not match its signed hash", asset.path));
    }
    let parent = path.parent().unwrap_or(dir);
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    replace(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// In the cache, checked by its signed hash, the cached copy included. Descriptive: `None` when missing,
/// each failure logged.
#[must_use]
pub fn icon_path(
    base: &str,
    dir: &Path,
    family: &Family,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Option<PathBuf> {
    let matches = |bytes: &[u8]| {
        hex(digest(&SHA256, bytes).as_ref()).eq_ignore_ascii_case(&family.icon_sha256)
    };
    let extension = Path::new(&family.icon).extension()?.to_str()?;
    let path = dir
        .join("icons")
        .join(format!("{}.{extension}", family.icon_sha256));
    if fs::read(&path).is_ok_and(|bytes| matches(&bytes)) {
        return Some(path);
    }
    let warn = |what: &str, error: &dyn std::fmt::Display| {
        log::warn!(target: "bonk::catalog", family = family.id.as_str(), error:% = error; "{what}");
    };
    let bytes = fetch(&format!("{base}{}", family.icon))
        .map_err(|e| warn("icon not downloaded", &e))
        .ok()?;
    if !matches(&bytes) {
        warn("icon does not match its hash", &family.icon_sha256);
        return None;
    }
    let saved = path
        .parent()
        .map_or(Ok(()), fs::create_dir_all)
        .and_then(|()| replace(&path, &bytes));
    if let Err(e) = saved {
        warn("icon not cached", &e);
        return None;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use ring::signature::{Ed25519KeyPair, KeyPair};

    use super::*;

    const NOW: u64 = 1_000;

    fn pair(seed: u8) -> Result<(Ed25519KeyPair, [u8; 32]), Box<dyn std::error::Error>> {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).map_err(|e| e.to_string())?;
        let public = pair.public_key().as_ref().try_into()?;
        Ok((pair, public))
    }

    fn manifest(sequence: u64, expires: u64, images: &str) -> Vec<u8> {
        format!(
            r#"{{"schema_version":1,"sequence":{sequence},"expires_unix":{expires},"families":[{}],"images":[{images}]}}"#,
            familles()
        )
        .into_bytes()
    }

    fn familles() -> String {
        let empreinte = "b".repeat(64);
        ["debian", "windows"]
            .map(|id| {
                format!(r#"{{"id":"{id}","name":"{id}","icon":"icons/{id}.svg","icon_sha256":"{empreinte}"}}"#)
            })
            .join(",")
    }

    fn debian() -> String {
        format!(
            r#"{{"id":"debian-13-netinst-x86_64","family":"debian","name":"Debian","version":"13.6","channel":"stable","released":"2026-01-10","edition":"netinst","arch":"x86_64","homepage":"https://www.debian.org/","url":"https://cdimage.debian.org/x.iso","sha256":"{}","size_bytes":123}}"#,
            "a".repeat(64)
        )
    }

    fn windows() -> String {
        r#"{"id":"windows-11-x86_64","family":"windows","name":"Windows","version":"11","channel":"stable","released":"2021-10-05","arch":"x86_64","homepage":"https://www.microsoft.com/","official_page":"https://www.microsoft.com/software-download/windows11"}"#.to_owned()
    }

    fn check(
        bytes: &[u8],
        key: &Ed25519KeyPair,
        keys: &[[u8; 32]],
        seen: u64,
    ) -> Result<Catalog, CatalogError> {
        let signature = key.sign(bytes);
        verify(
            bytes,
            signature.as_ref(),
            &Trust {
                keys,
                now_unix: NOW,
                seen_sequence: seen,
            },
        )
    }

    #[test]
    fn un_manifeste_signe_se_lit_avec_ses_deux_formes_d_image()
    -> Result<(), Box<dyn std::error::Error>> {
        let (key, public) = pair(1)?;
        let images = format!("{},{}", debian(), windows());
        let catalog = check(&manifest(5, NOW + 1, &images), &key, &[public], 5)?;
        assert_eq!(catalog.images.len(), 2);
        assert!(matches!(
            catalog.images[0].source,
            ImageSource::Download {
                size_bytes: 123,
                ..
            }
        ));
        assert!(matches!(
            catalog.images[1].source,
            ImageSource::OfficialPage { .. }
        ));
        Ok(())
    }

    #[test]
    fn signature_fausse_octet_change_ou_autre_cle_sont_refuses()
    -> Result<(), Box<dyn std::error::Error>> {
        let ((key, public), (autre, secours)) = (pair(1)?, pair(2)?);
        let bytes = manifest(1, NOW + 1, &debian());
        assert_eq!(
            check(&bytes, &autre, &[public], 0),
            Err(CatalogError::BadSignature)
        );
        assert!(check(&bytes, &autre, &[public, secours], 0).is_ok());

        let signature = key.sign(&bytes);
        let mut change = bytes.clone();
        change[20] ^= 1;
        let trust = Trust {
            keys: &[public],
            now_unix: NOW,
            seen_sequence: 0,
        };
        assert_eq!(
            verify(&change, signature.as_ref(), &trust),
            Err(CatalogError::BadSignature)
        );
        assert_eq!(verify(&bytes, &[], &trust), Err(CatalogError::BadSignature));
        // Without a key nothing is ever allowed.
        assert_eq!(check(&bytes, &key, &[], 0), Err(CatalogError::BadSignature));
        Ok(())
    }

    #[test]
    fn expire_ancien_ou_format_futur_sont_refuses_meme_signes()
    -> Result<(), Box<dyn std::error::Error>> {
        let (key, public) = pair(1)?;
        assert!(matches!(
            check(&manifest(1, NOW, &debian()), &key, &[public], 0),
            Err(CatalogError::Expired { .. })
        ));
        assert!(matches!(
            check(&manifest(4, NOW + 1, &debian()), &key, &[public], 5),
            Err(CatalogError::Rollback {
                sequence: 4,
                seen: 5
            })
        ));
        let futur = br#"{"schema_version":2,"autre_chose":true}"#;
        assert_eq!(
            check(futur, &key, &[public], 0),
            Err(CatalogError::UnsupportedVersion(2))
        );
        assert!(matches!(
            check(b"pas du json", &key, &[public], 0),
            Err(CatalogError::Unreadable(_))
        ));
        Ok(())
    }

    struct Dossier(PathBuf);

    impl Dossier {
        fn new(nom: &str) -> Self {
            Self(std::env::temp_dir().join(format!("bonk-catalog-{nom}-{}", std::process::id())))
        }
    }

    impl Drop for Dossier {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn la_copie_sert_si_le_reseau_manque_jamais_si_le_fichier_est_refuse()
    -> Result<(), Box<dyn std::error::Error>> {
        let (key, public) = pair(1)?;
        let dossier = Dossier::new("copie");
        let bytes = manifest(5, NOW + 1, &debian());
        let signature = key.sign(&bytes).as_ref().to_vec();
        let (m, s) = (bytes.clone(), signature.clone());
        let bon = move |url: &str| {
            Ok(if url.ends_with(".sig") {
                s.clone()
            } else {
                m.clone()
            })
        };
        let hors_ligne = |_: &str| Err("coupure".to_owned());
        let lire = |now, fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>| {
            load("https://c/", &dossier.0, now, &[public], fetch)
        };

        assert!(matches!(
            lire(NOW, &hors_ligne),
            Err(CatalogError::Unavailable(_))
        ));
        assert!(!lire(NOW, &bon)?.saved_copy);
        assert!(lire(NOW, &hors_ligne)?.saved_copy);

        // Received but tampered: refused, even with a good copy.
        let (mut f, s2) = (bytes.clone(), signature.clone());
        f[20] ^= 1;
        let falsifie = move |url: &str| {
            Ok(if url.ends_with(".sig") {
                s2.clone()
            } else {
                f.clone()
            })
        };
        assert_eq!(lire(NOW, &falsifie), Err(CatalogError::BadSignature));

        // An older manifest, even well signed, does not pass after a newer one.
        let ancien = manifest(4, NOW + 1, &debian());
        let s3 = key.sign(&ancien).as_ref().to_vec();
        let retour = move |url: &str| {
            Ok(if url.ends_with(".sig") {
                s3.clone()
            } else {
                ancien.clone()
            })
        };
        assert!(matches!(
            lire(NOW, &retour),
            Err(CatalogError::Rollback { .. })
        ));

        // The copy expires too.
        assert!(matches!(
            lire(NOW + 1, &hors_ligne),
            Err(CatalogError::Expired { .. })
        ));
        Ok(())
    }

    #[test]
    fn un_logo_faux_est_ecarte() -> Result<(), Box<dyn std::error::Error>> {
        let dossier = Dossier::new("logo");
        let contenu = b"<svg/>".to_vec();
        let empreinte = hex(digest(&SHA256, &contenu).as_ref());
        let family = |sha: String| Family {
            id: "ubuntu".to_owned(),
            name: "Ubuntu".to_owned(),
            category: None,
            icon: "icons/ubuntu.svg".to_owned(),
            icon_sha256: sha,
        };
        let fetch = move |_: &str| Ok(contenu.clone());
        assert_eq!(
            icon_path("https://c/", &dossier.0, &family("0".repeat(64)), &fetch),
            None
        );
        let chemin = icon_path("https://c/", &dossier.0, &family(empreinte.clone()), &fetch);
        assert!(chemin.is_some_and(|p| p.is_file()));
        // Already saved: no network.
        let hors_ligne = |_: &str| Err("coupure".to_owned());
        let chemin = icon_path(
            "https://c/",
            &dossier.0,
            &family(empreinte.clone()),
            &hors_ligne,
        );
        // A cached copy changed on disk is not served, even offline.
        std::fs::write(chemin.ok_or("en cache")?, b"<svg>autre</svg>")?;
        assert_eq!(
            icon_path(
                "https://c/",
                &dossier.0,
                &family(empreinte.clone()),
                &hors_ligne
            ),
            None
        );
        let reprise = icon_path("https://c/", &dossier.0, &family(empreinte), &fetch);
        assert_eq!(std::fs::read(reprise.ok_or("retéléchargé")?)?, b"<svg/>");
        Ok(())
    }

    fn avec_grub(grub: &str) -> Vec<u8> {
        let mut bytes = manifest(1, NOW + 1, &debian());
        bytes.truncate(bytes.len() - 1);
        bytes.extend(format!(r#","grub":{grub}}}"#).into_bytes());
        bytes
    }

    fn grub(efi_path: &str) -> String {
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        format!(
            r#"{{"version":"2.14","released":"2026-01-14","build":"6e330c761ee82877","efi":{{"x86_64":{{"path":"{efi_path}","sha256":"{a}","size_bytes":1}}}},"source":{{"path":"grub/2.14/grub-2.14.tar.xz","sha256":"{b}","size_bytes":2}},"source_signature":{{"path":"grub/2.14/grub-2.14.tar.xz.sig","sha256":"{b}","size_bytes":3}}}}"#
        )
    }

    #[test]
    fn l_entree_grub_se_lit_et_reste_dans_le_dossier_du_catalogue()
    -> Result<(), Box<dyn std::error::Error>> {
        let (key, public) = pair(1)?;
        let catalog = check(&manifest(1, NOW + 1, &debian()), &key, &[public], 0)?;
        assert_eq!(catalog.grub, None);
        let catalog = check(
            &avec_grub(&grub("grub/2.14/BOOTX64.EFI")),
            &key,
            &[public],
            0,
        )?;
        let entry = catalog.grub.ok_or("pas d'entrée grub")?;
        assert_eq!(entry.efi["x86_64"].path, "grub/2.14/BOOTX64.EFI");
        for chemin in [
            "../BOOTX64.EFI",
            "/BOOTX64.EFI",
            "https://autre/BOOTX64.EFI",
        ] {
            assert!(matches!(
                check(&avec_grub(&grub(chemin)), &key, &[public], 0),
                Err(CatalogError::InvalidGrub(_))
            ));
        }
        Ok(())
    }

    #[test]
    fn un_fichier_publie_est_controle_puis_garde_en_cache() -> Result<(), String> {
        let dossier = Dossier::new("asset");
        let contenu = b"MZ grub".to_vec();
        let asset = Asset {
            path: "grub/2.14/BOOTX64.EFI".to_owned(),
            sha256: hex(digest(&SHA256, &contenu).as_ref()),
            size_bytes: 7,
        };
        let faux = |_: &str, _: u64| Ok(b"autre".to_vec());
        assert!(asset_path("https://c/", &dossier.0, "grub", &asset, &faux).is_err());
        let servi = |url: &str, limite: u64| {
            assert_eq!(url, "https://c/grub/2.14/BOOTX64.EFI");
            assert_eq!(limite, 7);
            Ok(contenu.clone())
        };
        let chemin = asset_path("https://c/", &dossier.0, "grub", &asset, &servi)?;
        assert_eq!(fs::read(&chemin).map_err(|e| e.to_string())?, contenu);
        let hors_ligne = |_: &str, _: u64| Err("coupure".to_owned());
        assert_eq!(
            asset_path("https://c/", &dossier.0, "grub", &asset, &hors_ligne)?,
            chemin
        );
        // A cached copy changed on disk is not trusted.
        fs::write(&chemin, b"abime").map_err(|e| e.to_string())?;
        assert!(asset_path("https://c/", &dossier.0, "grub", &asset, &hors_ligne).is_err());
        Ok(())
    }

    #[test]
    fn les_cles_embarquees_sont_distinctes_et_refusent_une_signature_de_test()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_ne!(TRUSTED_KEYS[0], TRUSTED_KEYS[1]);
        let (key, _) = pair(1)?;
        assert_eq!(
            check(&manifest(1, NOW + 1, &debian()), &key, &TRUSTED_KEYS, 0),
            Err(CatalogError::BadSignature)
        );
        Ok(())
    }

    #[test]
    fn plusieurs_versions_d_un_systeme_et_cycle_de_vie() -> Result<(), Box<dyn std::error::Error>> {
        let (key, public) = pair(1)?;
        let ancienne = debian()
            .replace("debian-13-netinst", "debian-12-netinst")
            .replace("13.6", "12.11")
            .replace(r#""channel":"stable""#, r#""channel":"end_of_life""#);
        let images = format!("{},{ancienne}", debian());
        let catalog = check(&manifest(1, NOW + 1, &images), &key, &[public], 0)?;
        assert_eq!(catalog.images[1].channel, Channel::EndOfLife);
        assert_eq!(catalog.images[1].family, catalog.images[0].family);
        Ok(())
    }

    #[test]
    fn un_miroir_est_https_ou_onion() {
        assert!(is_mirror("https://m/x.iso"));
        assert!(is_mirror("http://abc.onion/x.iso"));
        assert!(!is_mirror("http://m/x.iso"));
        assert!(!is_mirror("http://onion.evil/abc.onion"));
        assert!(!is_mirror("ftp://abc.onion/x.iso"));
    }

    #[test]
    fn un_logo_ne_sort_pas_du_dossier_du_catalogue() {
        let family = |icon: &str| Family {
            id: "ubuntu".to_owned(),
            name: "Ubuntu".to_owned(),
            category: None,
            icon: icon.to_owned(),
            icon_sha256: "b".repeat(64),
        };
        assert_eq!(check_family(&family("icons/ubuntu.svg")), Ok(()));
        for refuse in [
            "../x.svg",
            "/x.svg",
            "https://x/y.svg",
            "a//b.svg",
            r"a\b.svg",
            "",
        ] {
            assert!(check_family(&family(refuse)).is_err(), "{refuse}");
        }
    }

    #[test]
    fn une_image_invalide_refuse_tout_le_manifeste() -> Result<(), Box<dyn std::error::Error>> {
        let (key, public) = pair(1)?;
        let cas = [
            debian().replace("https://cdimage", "http://cdimage"),
            debian().replace(&"a".repeat(64), "abc"),
            debian().replace("debian-13", "Debian 13"),
            debian().replace("https://www.debian.org/", "http://www.debian.org/"),
            windows().replace("https://www.microsoft.com/software", "ftp://x/software"),
            format!("{},{}", debian(), debian()),
            debian().replace(r#""family":"debian""#, r#""family":"absente""#),
            debian().replace("2026-01-10", "10 janvier"),
        ];
        for images in cas {
            assert!(
                matches!(
                    check(&manifest(1, NOW + 1, &images), &key, &[public], 0),
                    Err(CatalogError::InvalidImage { .. })
                ),
                "{images}"
            );
        }
        Ok(())
    }
}
