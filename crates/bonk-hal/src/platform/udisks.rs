use std::collections::HashMap;
use std::io;
use std::os::fd::OwnedFd;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::Duration;

use bonk_core::BlockError;
use zbus::{Connection, OwnedObjectPath, Variant, block_on};

const SERVICE: &str = "org.freedesktop.UDisks2";
const MANAGER_PATH: &str = "/org/freedesktop/UDisks2/Manager";
const MANAGER: &str = "org.freedesktop.UDisks2.Manager";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const SUDO_HINT: &str = "run again with sudo";
/// An unanswered authorization request is abandoned after this: nobody may be at the screen, and the
/// operation must not wait forever.
const AUTHORIZATION_LIMIT: Duration = Duration::from_secs(5 * 60);
const AUTHORIZATION_POLL: Duration = Duration::from_millis(100);

type Options<'a> = HashMap<&'a str, Variant<'a>>;

/// `None`: no known remedy.
fn advice(error_name: &str) -> Option<String> {
    match error_name {
        "org.freedesktop.UDisks2.Error.NotAuthorizedDismissed" => {
            Some("authorization cancelled".to_owned())
        }
        "org.freedesktop.UDisks2.Error.NotAuthorized"
        | "org.freedesktop.UDisks2.Error.NotAuthorizedCanObtain" => Some(format!(
            "authorization impossible: no polkit agent in this session (SSH?); {SUDO_HINT}"
        )),
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner" => {
            Some(format!("UDisks2 service missing; {SUDO_HINT}"))
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
                "UDisks2: {}: {}",
                name.as_str(),
                detail.as_deref().unwrap_or("no detail")
            )
        }));
    }
    denied(format!("UDisks2 unreachable: {error}; {SUDO_HINT}"))
}

fn connect() -> Result<Connection, BlockError> {
    block_on(Connection::system()).map_err(from_zbus)
}

fn resolve(connection: &Connection, device: &Path) -> Result<OwnedObjectPath, BlockError> {
    let path = device
        .to_str()
        .ok_or_else(|| denied("unreadable device name".to_owned()))?;
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
        .ok_or_else(|| denied(format!("{} unknown to UDisks2", device.display())))
}

/// For a partition the kernel just created: the service finds it after the kernel. Seen on the test key:
/// without waiting, `ResolveDevice` does not know it yet.
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

/// `mode`: `r` or `rw`; `flags`: `open(2)` flags without the mode. The call waits for the user's polkit
/// answer on a separate thread, abandoned on cancel (`ErrorKind::Interrupted`) or after
/// [`AUTHORIZATION_LIMIT`].
pub(super) fn open(
    device: &Path,
    mode: &str,
    flags: i32,
    cancelled: &dyn Fn() -> bool,
) -> Result<OwnedFd, BlockError> {
    let (sender, answer) = mpsc::channel();
    let (device, mode) = (device.to_path_buf(), mode.to_owned());
    std::thread::spawn(move || {
        // An answer after giving up is dropped: the received descriptor closes.
        let _ = sender.send(open_now(&device, &mode, flags));
    });
    let polls = u32::try_from(AUTHORIZATION_LIMIT.as_millis() / AUTHORIZATION_POLL.as_millis())
        .unwrap_or(u32::MAX);
    match wait_answer(&answer, cancelled, AUTHORIZATION_POLL, polls) {
        Waited::Answer(result) => result,
        Waited::Cancelled => Err(BlockError::Io(io::Error::new(
            io::ErrorKind::Interrupted,
            "cancelled during the authorization request",
        ))),
        Waited::TimedOut => Err(denied(format!(
            "no answer to the authorization request within {} minutes",
            AUTHORIZATION_LIMIT.as_secs() / 60
        ))),
        Waited::Lost => Err(denied("UDisks2: response lost".to_owned())),
    }
}

enum Waited<T> {
    Answer(T),
    Cancelled,
    TimedOut,
    Lost,
}

fn wait_answer<T>(
    answer: &Receiver<T>,
    cancelled: &dyn Fn() -> bool,
    poll: Duration,
    polls: u32,
) -> Waited<T> {
    for _ in 0..polls {
        if cancelled() {
            return Waited::Cancelled;
        }
        match answer.recv_timeout(poll) {
            Ok(value) => return Waited::Answer(value),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Waited::Lost,
        }
    }
    Waited::TimedOut
}

fn open_now(device: &Path, mode: &str, flags: i32) -> Result<OwnedFd, BlockError> {
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

/// Otherwise the message says what is missing.
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
        Err(format!("tool missing: {tool}"))
    }
}

/// Through the service, which runs `mkfs.*`. `take_ownership`: the filesystem belongs to the caller,
/// otherwise an ext4 partition would be writable by root only.
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

/// Through the service: the system call needs rights the caller lacks, and the kernel only does it in the
/// background after an unbounded delay.
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

/// Through the service, when `umount` is not allowed.
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
            Some("authorization cancelled")
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

#[cfg(test)]
mod wait_tests {
    use super::*;

    const POLL: Duration = Duration::from_millis(1);

    #[test]
    fn une_reponse_arrivee_est_rendue() {
        let (sender, answer) = mpsc::channel();
        sender.send(7).unwrap_or_default();
        assert!(matches!(
            wait_answer(&answer, &|| false, POLL, 3),
            Waited::Answer(7)
        ));
    }

    #[test]
    fn l_annulation_interrompt_l_attente() {
        let (_sender, answer) = mpsc::channel::<u8>();
        let calls = std::cell::Cell::new(0);
        let cancelled = || {
            calls.set(calls.get() + 1);
            calls.get() > 2
        };
        assert!(matches!(
            wait_answer(&answer, &cancelled, POLL, 100),
            Waited::Cancelled
        ));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn sans_reponse_l_attente_s_arrete_au_nombre_de_tours() {
        let (_sender, answer) = mpsc::channel::<u8>();
        assert!(matches!(
            wait_answer(&answer, &|| false, POLL, 3),
            Waited::TimedOut
        ));
    }

    #[test]
    fn un_fil_arrete_sans_reponse_est_signale() {
        let (sender, answer) = mpsc::channel::<u8>();
        drop(sender);
        assert!(matches!(
            wait_answer(&answer, &|| false, POLL, 3),
            Waited::Lost
        ));
    }
}
