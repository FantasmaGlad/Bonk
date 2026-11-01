//! Le catalogue des images : un fichier statique signé, publié hors du binaire.
//!
//! Le fichier change sans publier l'application, et l'hébergeur n'a pas à être cru : seule la signature,
//! contrôlée sur les octets exacts avec une clé embarquée dans le binaire, donne confiance. Rien n'est lu
//! avant ce contrôle. Fonctions pures : le réseau et l'horloge viennent de l'appelant.

use std::collections::HashSet;
use std::fmt;

use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

/// Version du format comprise par ce binaire.
pub const SCHEMA_VERSION: u32 = 1;

/// Une image proposée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    /// Identifiant stable : `debian-13-netinst-x86_64`.
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub edition: Option<String>,
    pub arch: String,
    pub homepage: String,
    #[serde(flatten)]
    pub source: ImageSource,
}

/// Où trouver l'image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ImageSource {
    /// Lien officiel, empreinte relue chez l'éditeur.
    Download {
        url: String,
        sha256: String,
        size_bytes: u64,
    },
    /// Éditeur sans lien stable (Windows) : l'utilisateur télécharge lui-même.
    OfficialPage { official_page: String },
}

/// Le manifeste, une fois vérifié.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub schema_version: u32,
    /// Croît à chaque publication : un miroir ne peut pas resservir un ancien manifeste à qui a vu plus récent.
    pub sequence: u64,
    pub expires_unix: u64,
    pub images: Vec<Image>,
}

/// Pourquoi un manifeste est refusé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    BadSignature,
    Unreadable(String),
    UnsupportedVersion(u32),
    Expired { expires_unix: u64 },
    Rollback { sequence: u64, seen: u64 },
    InvalidImage { id: String, reason: &'static str },
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

fn check_image(image: &Image) -> Result<(), &'static str> {
    let id_ok = !image.id.is_empty()
        && image.id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.' || b == b'_'
        });
    if !id_ok {
        return Err("identifier must be lowercase ASCII");
    }
    if !image.homepage.starts_with("https://") {
        return Err("homepage must be https");
    }
    match &image.source {
        ImageSource::Download { url, sha256, .. } => {
            if !url.starts_with("https://") {
                return Err("url must be https");
            }
            if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
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
    let mut ids = HashSet::new();
    for image in &catalog.images {
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
            r#"{{"schema_version":1,"sequence":{sequence},"expires_unix":{expires},"images":[{images}]}}"#
        )
        .into_bytes()
    }

    fn debian() -> String {
        format!(
            r#"{{"id":"debian-13-netinst-x86_64","name":"Debian","version":"13.6","edition":"netinst","arch":"x86_64","homepage":"https://www.debian.org/","url":"https://cdimage.debian.org/x.iso","sha256":"{}","size_bytes":123}}"#,
            "a".repeat(64)
        )
    }

    fn windows() -> String {
        r#"{"id":"windows-11-x86_64","name":"Windows","version":"11","arch":"x86_64","homepage":"https://www.microsoft.com/","official_page":"https://www.microsoft.com/software-download/windows11"}"#.to_owned()
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
