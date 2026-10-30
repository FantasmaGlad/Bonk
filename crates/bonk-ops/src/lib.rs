//! Opérations de Bonk, sans interface : la porte d'écriture unique et les codes d'erreur stables.
//! Toute interface passe par ce crate, qui ne dépend donc ni de clap ni d'Iced.

pub mod errors;
pub mod job;
