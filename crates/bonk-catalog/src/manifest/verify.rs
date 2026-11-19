use crate::sums::is_sha256;
use std::collections::HashSet;

use super::*;
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::Deserialize;

/// `YYYY-MM-DD` shape only: enough for text order to be date order.
pub(super) fn is_date(text: &str) -> bool {
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
pub(super) fn is_relative(path: &str) -> bool {
    !path.starts_with('/')
        && !path.contains('\\')
        && !path.contains(':')
        && path.split('/').all(|part| !part.is_empty() && part != "..")
}

pub(super) fn check_grub(grub: &Grub) -> Result<(), &'static str> {
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

pub(super) fn check_family(family: &Family) -> Result<(), &'static str> {
    if !crate::link::is_image_id(&family.id) {
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
pub(super) fn is_mirror(address: &str) -> bool {
    let onion = address
        .strip_prefix("http://")
        .and_then(|rest| rest.split('/').next())
        .is_some_and(|host| host.ends_with(".onion"));
    address.starts_with("https://") || onion
}

pub(super) fn check_image(image: &Image) -> Result<(), &'static str> {
    if !crate::link::is_image_id(&image.id) {
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
    if image
        .file_name
        .as_deref()
        .is_some_and(|name| !crate::is_plain_file_name(name))
    {
        return Err("file name must be one plain path segment");
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
    let catalog = parse(manifest)?;
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
    check_entries(&catalog)?;
    if let Some(grub) = &catalog.grub {
        check_grub(grub).map_err(CatalogError::InvalidGrub)?;
    }
    Ok(catalog)
}

/// Version first: a newer format may not parse as this one.
pub(super) fn parse(manifest: &[u8]) -> Result<Catalog, CatalogError> {
    #[derive(Deserialize)]
    struct Header {
        schema_version: u32,
    }
    let unreadable = |e: serde_json::Error| CatalogError::Unreadable(e.to_string());
    let header: Header = serde_json::from_slice(manifest).map_err(unreadable)?;
    if header.schema_version != SCHEMA_VERSION {
        return Err(CatalogError::UnsupportedVersion(header.schema_version));
    }
    serde_json::from_slice(manifest).map_err(unreadable)
}

pub(super) fn invalid(id: &str, reason: &'static str) -> CatalogError {
    CatalogError::InvalidImage {
        id: id.to_owned(),
        reason,
    }
}

/// Unique, valid families, and images that are unique, valid and of a known family.
pub(super) fn check_entries(catalog: &Catalog) -> Result<(), CatalogError> {
    let mut families = HashSet::new();
    for family in &catalog.families {
        check_family(family).map_err(|reason| invalid(&family.id, reason))?;
        if !families.insert(family.id.as_str()) {
            return Err(invalid(&family.id, "duplicate identifier"));
        }
    }
    let mut ids = HashSet::new();
    for image in &catalog.images {
        if !families.contains(image.family.as_str()) {
            return Err(invalid(&image.id, "unknown family"));
        }
        check_image(image).map_err(|reason| invalid(&image.id, reason))?;
        if !ids.insert(image.id.as_str()) {
            return Err(invalid(&image.id, "duplicate identifier"));
        }
    }
    Ok(())
}
