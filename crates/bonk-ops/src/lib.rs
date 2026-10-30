//! Opérations de Bonk, sans interface : la porte d'écriture unique (invariant 10) et les codes d'erreur
//! stables. Toute interface, ligne de commande, service ou application mobile, passe par ce crate ; il
//! ne dépend donc ni de clap ni d'Iced.

pub mod errors;
pub mod job;
