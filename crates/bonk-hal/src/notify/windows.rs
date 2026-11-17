use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_TIP, NIIF_INFO, NIIF_NOSOUND, NIIF_USER, NIM_ADD, NIM_DELETE,
    NIM_MODIFY, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, HWND_MESSAGE, IDI_APPLICATION, LoadIconW,
};

use super::{Notice, Notifier};

/// The shell's balloon, which Windows 10 and 11 show as a notification: no library and no application id,
/// so the portable executable has it too. It needs an icon in the notification area while it works.
#[derive(Default)]
pub(super) struct SystemNotifier {
    window: Option<HWND>,
    /// A balloon per percent would pile up in the notification centre: only a new step or the end.
    title: String,
}

impl SystemNotifier {
    fn data(window: HWND) -> NOTIFYICONDATAW {
        // SAFETY: NOTIFYICONDATAW is plain data; all zero is its documented empty value.
        let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        data.cbSize = u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(u32::MAX);
        data.hWnd = window;
        data.uID = 1;
        data
    }

    fn window(&mut self) -> Option<HWND> {
        if self.window.is_none() {
            let class = wide("STATIC");
            // SAFETY: a message-only window of a system class, owned by this thread, destroyed in `idle`.
            let window = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    std::ptr::null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            };
            if window.is_null() {
                log::info!(target: "bonk::hal", "no window for the notification icon");
                return None;
            }
            let mut data = Self::data(window);
            data.uFlags = NIF_ICON | NIF_TIP;
            // SAFETY: IDI_APPLICATION is a system icon, loaded without a module.
            data.hIcon = unsafe { LoadIconW(std::ptr::null_mut(), IDI_APPLICATION) };
            copy(&mut data.szTip, "Bonk");
            // SAFETY: `data` is initialised above and names this thread's window.
            if unsafe { Shell_NotifyIconW(NIM_ADD, &raw const data) } == 0 {
                log::info!(target: "bonk::hal", "notification icon refused");
            }
            self.window = Some(window);
        }
        self.window
    }
}

impl Notifier for SystemNotifier {
    fn show(&mut self, notice: &Notice) {
        if !notice.last && notice.title == self.title {
            return;
        }
        self.title = if notice.last {
            String::new()
        } else {
            notice.title.clone()
        };
        let Some(window) = self.window() else {
            return;
        };
        let mut data = Self::data(window);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = if notice.last {
            NIIF_INFO | NIIF_USER
        } else {
            NIIF_INFO | NIIF_NOSOUND
        };
        copy(&mut data.szInfoTitle, &notice.title);
        copy(&mut data.szInfo, &notice.body);
        // SAFETY: `data` names the icon added for this window.
        if unsafe { Shell_NotifyIconW(NIM_MODIFY, &raw const data) } == 0 {
            log::info!(target: "bonk::hal", "notification refused");
        }
    }

    fn idle(&mut self) {
        if let Some(window) = self.window.take() {
            let data = Self::data(window);
            // SAFETY: the icon and the window were created by this thread and are removed once.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &raw const data);
                DestroyWindow(window);
            }
        }
        self.title.clear();
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Truncated to the field, always NUL-terminated.
fn copy(field: &mut [u16], text: &str) {
    let room = field.len().saturating_sub(1);
    let mut length = 0;
    for (slot, unit) in field.iter_mut().zip(text.encode_utf16().take(room)) {
        *slot = unit;
        length += 1;
    }
    field[length] = 0;
}
