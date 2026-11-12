#![cfg_attr(
    test,
    allow(clippy::too_many_lines, reason = "a scenario test reads in one piece")
)]

pub mod bench;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod jni_api;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod screen;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod settings;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod vault;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod writer;
