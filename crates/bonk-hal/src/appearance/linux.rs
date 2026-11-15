use std::time::Duration;

use zbus::{Connection, block_on, connection};
use zbus::{OwnedVariant, Variant};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
const KEY: (&str, &str) = ("org.freedesktop.appearance", "color-scheme");
// A missing or stuck portal must not hold the window back.
const TIMEOUT: Duration = Duration::from_millis(500);

pub(super) fn prefers_dark() -> Option<bool> {
    match block_on(read_color_scheme()) {
        Ok(value) => scheme_is_dark(&value),
        Err(error) => {
            log::info!(target: "bonk::hal", error:% = error; "no colour scheme from the desktop portal");
            None
        }
    }
}

async fn read_color_scheme() -> zbus::Result<OwnedVariant> {
    let connection = connection::Builder::session()
        .method_timeout(TIMEOUT)
        .build()
        .await?;
    match read(&connection, "ReadOne").await {
        Ok(value) => Ok(value),
        // ReadOne came with version 2 of the interface; older portals only have Read.
        Err(_) => read(&connection, "Read").await,
    }
}

async fn read(connection: &Connection, method: &str) -> zbus::Result<OwnedVariant> {
    let reply = connection
        .call_method(Some(PORTAL), PORTAL_PATH, Some(SETTINGS), method, &KEY)
        .await?;
    reply.body().deserialize()
}

/// XDG portal values: 0 no preference, 1 dark, 2 light; `Read` wraps the value in one more variant.
fn scheme_is_dark(value: &Variant<'_>) -> Option<bool> {
    match value {
        Variant::U32(1) => Some(true),
        Variant::U32(0 | 2) => Some(false),
        Variant::Variant(inner) => scheme_is_dark(inner),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_valeur_du_portail_dit_clair_ou_sombre() {
        assert_eq!(scheme_is_dark(&Variant::U32(1)), Some(true));
        assert_eq!(scheme_is_dark(&Variant::U32(2)), Some(false));
        assert_eq!(scheme_is_dark(&Variant::U32(0)), Some(false));
        assert_eq!(scheme_is_dark(&Variant::U32(7)), None);
        assert_eq!(scheme_is_dark(&Variant::from("dark")), None);
    }

    #[test]
    fn la_valeur_enveloppee_par_read_est_lue() {
        let wrapped = Variant::Variant(Box::new(Variant::U32(1)));
        assert_eq!(scheme_is_dark(&wrapped), Some(true));
    }
}
