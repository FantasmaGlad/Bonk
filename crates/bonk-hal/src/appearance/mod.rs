#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as current;
#[cfg(target_os = "macos")]
use macos as current;
#[cfg(windows)]
use windows as current;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod current {
    pub(super) fn prefers_dark() -> Option<bool> {
        None
    }
}

#[must_use]
pub fn system_prefers_dark() -> Option<bool> {
    current::prefers_dark()
}
