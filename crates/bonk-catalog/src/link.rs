use std::fmt;

/// The only link Bonk answers: it names an image of the signed catalog, never an address or a disk.
const PREFIX: &str = "bonk://install/";

const MAX_ID: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkError {
    NotAnInstallLink,
    InvalidImageId,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotAnInstallLink => "not a bonk://install/<image> link",
            Self::InvalidImageId => {
                "invalid image identifier: 1 to 128 characters among a-z, 0-9, '.', '_' and '-'"
            }
        })
    }
}

impl std::error::Error for LinkError {}

/// A link comes from a web page: anything outside the catalog's own alphabet is refused, so it can carry
/// neither a path, a query nor an option.
#[must_use]
pub fn is_image_id(id: &str) -> bool {
    (1..=MAX_ID).contains(&id.len())
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
        && !id.starts_with('-')
}

/// The catalog identifier named by `bonk://install/<id>`.
pub fn image_id(link: &str) -> Result<&str, LinkError> {
    let id = link
        .get(..PREFIX.len())
        .filter(|start| start.eq_ignore_ascii_case(PREFIX))
        .map(|_| &link[PREFIX.len()..])
        .ok_or(LinkError::NotAnInstallLink)?;
    if is_image_id(id) {
        Ok(id)
    } else {
        Err(LinkError::InvalidImageId)
    }
}

#[cfg(test)]
mod tests {
    use super::{LinkError, image_id, is_image_id};

    #[test]
    fn un_lien_d_installation_donne_l_identifiant_du_catalogue() {
        assert_eq!(
            image_id("bonk://install/ubuntu-26.04-desktop-x86_64"),
            Ok("ubuntu-26.04-desktop-x86_64")
        );
        assert_eq!(image_id("BONK://Install/debian-13"), Ok("debian-13"));
    }

    #[test]
    fn tout_autre_lien_est_refuse() {
        for link in [
            "",
            "bonk://",
            "bonk://write/ubuntu",
            "bonk:install/ubuntu",
            "https://bonk-os.app/install/ubuntu",
            "bonk://install",
            "é",
        ] {
            assert_eq!(image_id(link), Err(LinkError::NotAnInstallLink), "{link}");
        }
    }

    #[test]
    fn un_identifiant_ne_porte_ni_chemin_ni_requete_ni_option() {
        for link in [
            "bonk://install/",
            "bonk://install/ubuntu/",
            "bonk://install/../etc",
            "bonk://install/ubuntu?disk=sda",
            "bonk://install/ubuntu#x",
            "bonk://install/ubuntu%20x",
            "bonk://install/Ubuntu",
            "bonk://install/--yes",
            "bonk://install/a b",
            "bonk://install/https://x/y.iso",
        ] {
            assert_eq!(image_id(link), Err(LinkError::InvalidImageId), "{link}");
        }
        assert!(!is_image_id(&"a".repeat(129)));
        assert!(is_image_id(&"a".repeat(128)));
    }
}
