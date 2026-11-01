//! Lecture des fichiers d'empreintes publiés par les distributions (`SHA256SUMS`).

/// Empreinte SHA-256 de `name` dans un fichier d'empreintes, en minuscules. Formats reconnus : celui de
/// `sha256sum` (`hex  nom`, `hex *nom`, `./nom`) et le format BSD (`SHA256 (nom) = hex`). Une ligne d'un autre
/// format est ignorée : ces fichiers portent parfois des commentaires ou une signature en clair.
#[must_use]
pub fn find_sha256(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (hex, entry) = parse_line(line.trim())?;
        (entry.strip_prefix("./").unwrap_or(entry) == name && is_sha256(hex))
            .then(|| hex.to_ascii_lowercase())
    })
}

fn parse_line(line: &str) -> Option<(&str, &str)> {
    if let Some(rest) = line.strip_prefix("SHA256 (") {
        let (entry, hex) = rest.rsplit_once(") = ")?;
        return Some((hex, entry));
    }
    let (hex, entry) = line.split_once(char::is_whitespace)?;
    let entry = entry.trim_start();
    Some((hex, entry.strip_prefix('*').unwrap_or(entry)))
}

/// Une empreinte SHA-256 écrite en hexadécimal.
#[must_use]
pub fn is_sha256(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "ABCDEF0123456789abcdef0123456789abcdef0123456789abcdef0123456789";

    #[test]
    fn les_formats_publies_sont_reconnus() {
        let lower = HEX.to_ascii_lowercase();
        for text in [
            format!("{HEX}  debian.iso\n"),
            format!("{HEX} *debian.iso\n"),
            format!("{HEX}  ./debian.iso\n"),
            format!("SHA256 (debian.iso) = {HEX}\n"),
            format!("# commentaire\n0000  autre.iso\n{HEX}  debian.iso\n"),
        ] {
            assert_eq!(
                find_sha256(&text, "debian.iso").as_ref(),
                Some(&lower),
                "{text}"
            );
        }
    }

    #[test]
    fn un_autre_nom_ou_une_empreinte_mal_formee_ne_donne_rien() {
        let text = format!("{HEX}  autre.iso\nabcd  debian.iso\n");
        assert_eq!(find_sha256(&text, "debian.iso"), None);
        assert_eq!(
            find_sha256(&format!("{HEX}  debian.iso.torrent\n"), "debian.iso"),
            None
        );
    }
}
