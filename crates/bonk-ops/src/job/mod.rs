//! Les opérations sur un support : écrire, vérifier, réparer le GPT, formater.
//!
//! Porte d'écriture unique, partagée par la ligne de commande, le service (`bonk serve`) et tout autre
//! front : elle enchaîne les garde-fous dans l'ordre voulu, ne parle jamais à l'utilisateur, et rend
//! des résultats typés et sérialisables (d'où `bonk schema`) ou une [`JobError`] qui porte son code.
//!
//! Ordre d'une écriture sur un disque : le filtre juge la cible ; sans `--yes` ou `--force` rien ne
//! bouge, et un disque interne exige `--force` ; le droit d'ouvrir est obtenu ; les partitions sont
//! démontées ; **juste avant l'ouverture**, le disque est ré-énuméré et son identité revérifiée ; le
//! noyau refuse enfin un disque encore utilisé (`O_EXCL`). Les tests injectent ces dépendances
//! ([`Deps`]) et n'ouvrent jamais de vrai disque.

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
pub use source::{FileReader, ImageFile};
pub use types::*;
pub use verify::{VerifyJob, verify};
pub use write::{WriteJob, write};
