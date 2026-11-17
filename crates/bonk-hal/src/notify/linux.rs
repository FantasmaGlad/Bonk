use std::collections::HashMap;
use std::time::Duration;

use zbus::{Connection, Variant, block_on, connection};

use super::{Notice, Notifier};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
// A stuck notification daemon must not delay the next notice.
const TIMEOUT: Duration = Duration::from_secs(2);
/// Matches `bonk.desktop` and the window's `app_id`.
const DESKTOP_ENTRY: &str = "bonk";

/// The desktop notification service, freedesktop.org Desktop Notifications 1.2.
#[derive(Default)]
pub(super) struct SystemNotifier {
    connection: Option<Connection>,
    /// The daemon's id of the notification on screen: the next one replaces it.
    shown: u32,
    failed: bool,
}

impl SystemNotifier {
    fn connection(&mut self) -> Option<&Connection> {
        if self.connection.is_none() && !self.failed {
            match block_on(
                connection::Builder::session()
                    .method_timeout(TIMEOUT)
                    .build(),
            ) {
                Ok(connection) => self.connection = Some(connection),
                Err(error) => {
                    self.failed = true;
                    log::info!(target: "bonk::hal", error:% = error; "no session bus: no notifications");
                }
            }
        }
        self.connection.as_ref()
    }
}

impl Notifier for SystemNotifier {
    fn show(&mut self, notice: &Notice) {
        let replaces = self.shown;
        let Some(connection) = self.connection() else {
            return;
        };
        let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
        hints.insert("desktop-entry", Variant::from(DESKTOP_ENTRY));
        // Progress is low urgency; the end stays until the user sees it.
        hints.insert("urgency", Variant::U8(u8::from(notice.last)));
        if let Some(percent) = notice.percent {
            hints.insert("value", Variant::I32(i32::from(percent)));
        }
        let expire: i32 = if notice.last { 0 } else { -1 };
        let body = (
            "Bonk",
            replaces,
            DESKTOP_ENTRY,
            notice.title.as_str(),
            notice.body.as_str(),
            Vec::<&str>::new(),
            hints,
            expire,
        );
        let reply =
            block_on(connection.call_method(Some(SERVICE), PATH, Some(SERVICE), "Notify", &body));
        match reply.and_then(|reply| reply.body().deserialize::<u32>()) {
            Ok(id) => self.shown = if notice.last { 0 } else { id },
            Err(error) => {
                log::info!(target: "bonk::hal", error:% = error; "notification refused");
            }
        }
    }
}
