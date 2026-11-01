//! La langue de la fenêtre au premier lancement, tant que rien n'a été choisi.

use bonk_hal::Lang;

/// `fr`, `fr-FR`, `fr_CA.UTF-8` : français. Tout le reste, ou rien : anglais, la langue de la CLI.
fn from_locale(locale: Option<&str>) -> Lang {
    let primary = locale
        .and_then(|name| name.split(['-', '_', '.']).next())
        .unwrap_or_default();
    if primary.eq_ignore_ascii_case("fr") {
        Lang::Fr
    } else {
        Lang::En
    }
}

pub(crate) fn detect() -> Lang {
    from_locale(sys_locale::get_locale().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn francais_reconnu_et_anglais_par_defaut() {
        for name in ["fr", "fr-FR", "fr_CA.UTF-8", "FR"] {
            assert_eq!(from_locale(Some(name)), Lang::Fr, "{name}");
        }
        for name in ["en-US", "de_DE", "C", "POSIX", "free", ""] {
            assert_eq!(from_locale(Some(name)), Lang::En, "{name}");
        }
        assert_eq!(from_locale(None), Lang::En);
    }
}
