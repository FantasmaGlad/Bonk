use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

pub(super) fn prefers_dark() -> Option<bool> {
    let key = wide(PERSONALIZE);
    let name = wide("AppsUseLightTheme");
    let mut light = 0u32;
    let mut size = 4u32;
    // SAFETY: both names end with a NUL; `light` is a u32 and `size` gives its length in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&raw mut light).cast(),
            &raw mut size,
        )
    };
    if status != ERROR_SUCCESS {
        log::info!(target: "bonk::hal", status = status; "AppsUseLightTheme not read");
        return None;
    }
    Some(light == 0)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
