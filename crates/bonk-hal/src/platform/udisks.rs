//! UDisks2 : obtenir un disque sans lancer Bonk en administrateur (D-36).
//!
//! Le service du système ouvre le périphérique et nous rend un descripteur ; l'autorisation passe par
//! polkit, au moment où on en a besoin. Tout échec devient `PermissionDenied` avec une phrase qui dit
//! quoi faire : `bonk-ops` le traduit en `E4004`.

use std::collections::HashMap;
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::time::Duration;

use bonk_core::BlockError;
use zbus::{Connection, OwnedObjectPath, Variant, block_on};

const SERVICE: &str = "org.freedesktop.UDisks2";
const MANAGER_PATH: &str = "/org/freedesktop/UDisks2/Manager";
const MANAGER: &str = "org.freedesktop.UDisks2.Manager";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const SUDO_HINT: &str = "relancer avec sudo";

type Options<'a> = HashMap<&'a str, Variant<'a>>;

/// Traduit un nom d'erreur D-Bus en phrase ; `None` : erreur sans remède connu.
fn advice(error_name: &str) -> Option<String> {
    match error_name {
        "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed" => {
            Some("autorisation annulée".to_owned())
        }
        "org.freedesktop.UDisks2.Error.NotAuthorized"
        | "org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain" => Some(format!(
            "autorisation impossible : aucun agent polkit dans cette session (SSH ?) ; {SUDO_HINT}"
        )),
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner" => {
            Some(format!("service UDisks2 absent ; {SUDO_HINT}"))
        }
        _ => None,
    }
}

fn denied(message: String) -> BlockError {
    BlockError::Io(io::Error::new(io::ErrorKind::PermissionDenied, message))
}

fn from_zbus(error: zbus::Error) -> BlockError {
    if let zbus::Error::MethodError(name, detail, _) = &error {
        return denied(advice(name.as_str()).unwrap_or_else(|| {
            format!(
                "UDisks2 : {} : {}",
                name.as_str(),
                detail.as_deref().unwrap_or("sans détail")
            )
        }));
    }
    denied(format!("UDisks2 injoignable : {error} ; {SUDO_HINT}"))
}

fn connect() -> Result<Connection, BlockError> {
    block_on(Connection::system()).map_err(from_zbus)
}

/// L'objet UDisks2 d'un périphérique, d'après son chemin.
fn resolve(connection: &Connection, device: &Path) -> Result<OwnedObjectPath, BlockError> {
    let path = device
        .to_str()
        .ok_or_else(|| denied("nom de périphérique illisible".to_owned()))?;
    let spec: Options<'_> = HashMap::from([("path", Variant::from(path))]);
    let options: Options<'_> = HashMap::new();
    let reply = block_on(connection.call_method(
        Some(SERVICE),
        MANAGER_PATH,
        Some(MANAGER),
        "ResolveDevice",
        &(spec, options),
    ))
    .map_err(from_zbus)?;
    let objects: Vec<OwnedObjectPath> = reply.body().deserialize().map_err(from_zbus)?;
    objects
        .into_iter()
        .next()
        .ok_or_else(|| denied(format!("{} inconnu de UDisks2", device.display())))
}

/// Comme [`resolve`], pour une partition que le noyau vient de créer : le service la découvre après lui.
/// Constaté sur la clé de test : sans attente, `ResolveDevice` ne la connaît pas encore.
fn resolve_new(connection: &Connection, device: &Path) -> Result<OwnedObjectPath, BlockError> {
    const LOOKS: u32 = 40;
    const PAUSE: Duration = Duration::from_millis(250);
    let mut result = resolve(connection, device);
    for _ in 1..LOOKS {
        if result.is_ok() {
            break;
        }
        std::thread::sleep(PAUSE);
        result = resolve(connection, device);
    }
    result
}

/// Demande au service d'ouvrir `device`. `mode` : `r` ou `rw` ; `flags` : ceux d'`open(2)`, hors mode.
///
/// # Errors
/// `PermissionDenied` avec le remède, si le service refuse ou n'est pas joignable.
pub(super) fn open(device: &Path, mode: &str, flags: i32) -> Result<OwnedFd, BlockError> {
    let connection = connect()?;
    let object = resolve(&connection, device)?;
    let options: Options<'_> = HashMap::from([("flags", Variant::from(flags))]);
    let reply = block_on(connection.call_method(
        Some(SERVICE),
        object.as_str(),
        Some(BLOCK),
        "OpenDevice",
        &(mode, options),
    ))
    .map_err(from_zbus)?;
    let fd: zbus::wire::OwnedFd = reply.body().deserialize().map_err(from_zbus)?;
    Ok(fd.into())
}

/// Le service répond-il ?
///
/// # Errors
/// Le message dit ce qui manque.
pub(super) fn reachable() -> Result<(), String> {
    connect()
        .and_then(|connection| {
            block_on(connection.call_method(
                Some(SERVICE),
                MANAGER_PATH,
                Some("org.freedesktop.DBus.Peer"),
                "Ping",
                &(),
            ))
            .map_err(from_zbus)
        })
        .map(drop)
        .map_err(|error| error.to_string())
}

/// Le service sait-il formater ce type ? Il répond sans rien demander à polkit.
///
/// # Errors
/// Le message nomme l'outil qui manque, ou dit que le service n'est pas joignable.
pub(super) fn can_format(kind: &str) -> Result<(), String> {
    let connection = connect().map_err(|error| error.to_string())?;
    let reply = block_on(connection.call_method(
        Some(SERVICE),
        MANAGER_PATH,
        Some(MANAGER),
        "CanFormat",
        &(kind,),
    ))
    .map_err(|error| from_zbus(error).to_string())?;
    let (available, tool): (bool, String) = reply
        .body()
        .deserialize()
        .map_err(|error| from_zbus(error).to_string())?;
    if available {
        Ok(())
    } else {
        Err(format!("outil absent : {tool}"))
    }
}

/// Formate une partition par le service, qui appelle `mkfs.*`. `take_ownership` : le système de fichiers
/// appartiendra à l'appelant, sans quoi une partition ext4 ne serait écrivable que par root.
///
/// # Errors
/// `PermissionDenied` avec le remède, si le service refuse, échoue ou n'est pas joignable.
pub(super) fn format(
    partition: &Path,
    kind: &str,
    label: Option<&str>,
    take_ownership: bool,
    interactive: bool,
) -> Result<(), BlockError> {
    let connection = connect()?;
    let object = resolve_new(&connection, partition)?;
    let mut options: Options<'_> = HashMap::new();
    if let Some(label) = label {
        options.insert("label", Variant::from(label));
    }
    if take_ownership {
        options.insert("take-ownership", Variant::from(true));
    }
    if !interactive {
        options.insert("auth.no_user_interaction", Variant::from(true));
    }
    block_on(connection.call_method(
        Some(SERVICE),
        object.as_str(),
        Some(BLOCK),
        "Format",
        &(kind, options),
    ))
    .map_err(from_zbus)?;
    Ok(())
}

/// Fait relire la table de partitions par le service : l'appel système exige des droits que l'appelant n'a
/// pas, et le noyau ne le fait de lui-même qu'en tâche de fond, après un délai sans borne connue.
///
/// # Errors
/// `PermissionDenied` avec le remède, si le service refuse ou n'est pas joignable.
pub(super) fn rescan(disk: &Path, interactive: bool) -> Result<(), BlockError> {
    let connection = connect()?;
    let object = resolve(&connection, disk)?;
    let mut options: Options<'_> = HashMap::new();
    if !interactive {
        options.insert("auth.no_user_interaction", Variant::from(true));
    }
    block_on(connection.call_method(
        Some(SERVICE),
        object.as_str(),
        Some(BLOCK),
        "Rescan",
        &(options,),
    ))
    .map_err(from_zbus)?;
    Ok(())
}

/// Démonte une partition par le service, quand `umount` n'en a pas le droit.
///
/// # Errors
/// `PermissionDenied` avec le remède, si le service refuse ou n'est pas joignable.
pub(super) fn unmount(partition: &Path) -> Result<(), BlockError> {
    let connection = connect()?;
    let object = resolve(&connection, partition)?;
    let options: Options<'_> = HashMap::new();
    block_on(connection.call_method(
        Some(SERVICE),
        object.as_str(),
        Some(FILESYSTEM),
        "Unmount",
        &(options,),
    ))
    .map_err(from_zbus)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_refus_connu_dit_quoi_faire() {
        assert_eq!(
            advice("org.freedesktop.UDisks2.Error.NotAuthorizedDismissed").as_deref(),
            Some("autorisation annulée")
        );
        for name in [
            "org.freedesktop.UDisks2.Error.NotAuthorized",
            "org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain",
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
        ] {
            assert!(advice(name).is_some_and(|m| m.contains("sudo")), "{name}");
        }
        assert_eq!(advice("org.freedesktop.UDisks2.Error.Failed"), None);
    }
}
