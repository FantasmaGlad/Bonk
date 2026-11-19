/// A name from the network or the catalog must not escape the cache directory: one path segment, not
/// encoded.
#[must_use]
pub fn is_plain_file_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '%', '\\', '\0'])
        && !name.chars().any(char::is_control)
}

/// Last path segment, without query or fragment.
#[must_use]
pub fn file_name(url: &str) -> Option<&str> {
    let path = url.split(['?', '#']).next()?;
    let (_, after_scheme) = path.split_once("://")?;
    let (_, path) = after_scheme.split_once('/')?;
    path.rsplit('/')
        .next()
        .filter(|name| is_plain_file_name(name))
}

/// The name the catalog gives, else the one of the address: a share link ends in `download`.
#[must_use]
pub fn download_name<'a>(url: &'a str, given: Option<&'a str>) -> Option<&'a str> {
    match given {
        Some(name) => is_plain_file_name(name).then_some(name),
        None => file_name(url),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_nom_de_fichier_d_une_url_ne_sort_pas_du_dossier() {
        assert_eq!(
            file_name("https://a.org/x/debian.iso?y=1#z"),
            Some("debian.iso")
        );
        for url in [
            "https://a.org/",
            "https://a.org",
            "https://a.org/x/..",
            "https://a.org/x/%2e%2e%2fetc",
            "https://a.org/a\\b",
        ] {
            assert_eq!(file_name(url), None, "{url}");
        }
    }

    #[test]
    fn le_nom_du_catalogue_passe_avant_celui_de_l_adresse() {
        let share = "https://kdrive.example/share/files/686/download";
        assert_eq!(download_name(share, None), Some("download"));
        assert_eq!(
            download_name(share, Some("GLF-OS-STABLE.iso")),
            Some("GLF-OS-STABLE.iso")
        );
        for refused in ["", ".", "..", "a/b", "a\\b", "%2e", "a\u{0}b", "a\nb"] {
            assert_eq!(download_name(share, Some(refused)), None, "{refused:?}");
        }
    }
}
