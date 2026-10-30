//! Les opérations d'écriture : écrire, vérifier, réparer le GPT (CDC §3, §4, J1).
//!
//! Ce module est la porte d'écriture unique, partagée par la ligne de commande, le service
//! (`bonk serve`) et tout autre front : il enchaîne les garde-fous dans l'ordre voulu et ne parle jamais à l'utilisateur. Il rend
//! des résultats typés (et sérialisables, d'où `bonk schema`) ou une [`JobError`] qui porte son code.
//!
//! Ordre d'une écriture sur un disque : le filtre juge la cible ; sans `--yes` rien ne bouge ; les
//! partitions sont démontées ; **juste avant l'ouverture** le disque est ré-énuméré et son identité
//! revérifiée (CDC §3.3) ; le noyau refuse enfin un disque encore utilisé (`O_EXCL`). Les tests
//! injectent ces dépendances ([`Deps`]) et n'ouvrent jamais de vrai disque.

mod format;
mod repair;
mod source;
mod target;
mod translate;
mod types;
mod verify;
mod write;

pub use format::{FormatJob, default_filesystem, format};
pub use repair::{RepairJob, repair};
pub use source::open_source;
pub use types::*;
pub use verify::{VerifyJob, verify};
pub use write::{WriteJob, write};
