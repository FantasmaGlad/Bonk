//! Accès aux disques, propre à chaque système d'exploitation : énumération, filtre de sécurité,
//! accès brut, démontage, privilèges et formatage via les outils de l'OS.
//!
//! C'est le seul crate qui contient du code spécifique à un OS, isolé derrière `cfg(target_os)`
//! et une interface commune.
