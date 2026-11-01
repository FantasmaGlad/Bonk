//! Le catalogue des images : un fichier statique signé, publié hors du binaire.
//!
//! Le fichier change sans publier l'application, et l'hébergeur n'a pas à être cru : seule la signature,
//! contrôlée sur les octets exacts avec une clé embarquée dans le binaire, donne confiance. Rien n'est lu
//! avant ce contrôle. Fonctions pures : le réseau et l'horloge viennent de l'appelant.

use crate::sums::is_sha256;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use ring::digest::{SHA256, digest};
use ring::signature::{ED25519, UnparsedPublicKey};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Version du format comprise par ce binaire.
pub const SCHEMA_VERSION: u32 = 1;

/// Clés publiques admises : la courante, dans les secrets de publication, et celle de secours, hors ligne,
/// qui permet de continuer à publier sans nouvelle version de Bonk si la première est perdue.
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

/// Un système, tel que l'écran de choix le présente : ses versions sont ses images.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Family {
    /// Identifiant stable : `ubuntu`.
    pub id: String,
    pub name: String,
    /// Group shown to sort systems: `Debian derivatives`. Absent from older catalogs.
    #[serde(default)]
    pub category: Option<String>,
    /// Logo, chemin relatif au manifeste. Son empreinte, couverte par la signature, l'empêche d'être remplacé.
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

/// Où trouver l'image.
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

/// Les nombres d'une version : `24.04.5` passe après `26.04.1`, même sorti plus tard, et `9` avant `10`.
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

/// Le manifeste, une fois vérifié.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Catalog {
    pub schema_version: u32,
    /// Croît à chaque publication : un miroir ne peut pas resservir un ancien manifeste à qui a vu plus récent.
    pub sequence: u64,
    pub expires_unix: u64,
    pub families: Vec<Family>,
    pub images: Vec<Image>,
}

/// Pourquoi un manifeste est refusé.
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
    /// Ni le réseau ni une copie gardée.
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
            Self::Unavailable(detail) => write!(f, "catalog unavailable: {detail}"),
        }
    }
}

impl std::error::Error for CatalogError {}

/// Ce que l'appelant sait : les clés admises, l'heure, le plus grand numéro déjà vu.
#[derive(Debug, Clone, Copy)]
pub struct Trust<'a> {
    /// Clés publiques Ed25519 : la courante et celle de secours, pour survivre à une clé perdue.
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

/// `AAAA-MM-JJ` : la forme seule, assez pour que le tri par texte soit le tri par date.
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

fn check_family(family: &Family) -> Result<(), &'static str> {
    if !is_identifier(&family.id) {
        return Err("identifier must be lowercase ASCII");
    }
    // Reste sous le dossier du manifeste : ni chemin absolu, ni remontée, ni autre hôte.
    let relative = !family.icon.starts_with('/')
        && !family.icon.contains('\\')
        && !family.icon.contains(':')
        && family
            .icon
            .split('/')
            .all(|part| !part.is_empty() && part != "..");
    if !relative {
        return Err("icon must be a relative path");
    }
    if !is_sha256(&family.icon_sha256) {
        return Err("icon sha256 must be 64 hexadecimal characters");
    }
    Ok(())
}

/// Un service onion chiffre déjà tout le chemin : c'est le seul `http://` admis.
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

/// Contrôle la signature, puis seulement lit et valide le manifeste.
///
/// # Errors
/// Signature fausse, format illisible ou trop récent, manifeste expiré ou plus ancien que le dernier vu,
/// image invalide.
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
    // La version d'abord : un format plus récent peut ne pas se lire comme celui-ci.
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
    Ok(catalog)
}

/// Le catalogue vérifié, et d'où il vient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub catalog: Catalog,
    /// Copie gardée, le réseau ayant manqué. Elle reste soumise à la signature et à l'expiration.
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

/// Écrit sans jamais laisser un fichier à moitié : une coupure garde l'ancien.
fn replace(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let partial = path.with_extension("part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, path)
}

/// Lit le catalogue sous `base`, ou, **si le réseau seul a manqué**, la dernière copie vérifiée. Un fichier
/// reçu mais refusé reste une erreur : se rabattre en silence masquerait une falsification.
///
/// # Errors
/// Fichier refusé par [`verify`], ou ni réseau ni copie.
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
            // Le cache n'est qu'un secours : ne pas pouvoir l'écrire n'empêche pas de lire le catalogue.
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

/// Le logo d'un système, dans le cache, contrôlé par son empreinte signée. Descriptif : `None` s'il manque.
#[must_use]
pub fn icon_path(
    base: &str,
    dir: &Path,
    family: &Family,
    fetch: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Option<PathBuf> {
    let extension = Path::new(&family.icon).extension()?.to_str()?;
    let path = dir
        .join("icons")
        .join(format!("{}.{extension}", family.icon_sha256));
    if path.is_file() {
        return Some(path);
    }
    let bytes = fetch(&format!("{base}{}", family.icon)).ok()?;
    if !hex(digest(&SHA256, &bytes).as_ref()).eq_ignore_ascii_case(&family.icon_sha256) {
        log::warn!(target: "bonk::catalog", family = family.id.as_str(); "icon does not match its hash");
        return None;
    }
    fs::create_dir_all(path.parent()?).ok()?;
    replace(&path, &bytes).ok()?;
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
        // La clé de secours est admise.
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
        // Sans clé, rien n'est jamais admis.
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

    /// Dossier jetable, retiré à la fin du test.
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

        // Rien encore : ni réseau ni copie.
        assert!(matches!(
            lire(NOW, &hors_ligne),
            Err(CatalogError::Unavailable(_))
        ));
        assert!(!lire(NOW, &bon)?.saved_copy);
        assert!(lire(NOW, &hors_ligne)?.saved_copy);

        // Reçu mais falsifié : refusé, même avec une bonne copie.
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

        // Un ancien manifeste, même bien signé, ne repasse pas après un plus récent.
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

        // La copie expire aussi.
        assert!(matches!(
            lire(NOW + 1, &hors_ligne),
            Err(CatalogError::Expired { .. })
        ));
        Ok(())
    }

    #[test]
    fn un_logo_faux_est_ecarte() {
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
        // Déjà gardé : pas de réseau.
        let hors_ligne = |_: &str| Err("coupure".to_owned());
        assert!(icon_path("https://c/", &dossier.0, &family(empreinte), &hors_ligne).is_some());
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
