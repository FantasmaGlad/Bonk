//! Bibliothèque native de l'application Android : pilote une clé USB par le descripteur que donne le système,
//! sans root. L'application réclame l'interface et garde le descripteur ; ici, on ne fait que des transferts.

pub mod bench;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod jni_api;
#[cfg(any(target_os = "linux", target_os = "android"))]
pub mod screen;
#[cfg(any(target_os = "linux", target_os = "android"))]
mod writer;
