//! Commandes `bonk write`, `bonk verify` et `bonk repair-gpt` : arguments, progression, affichage.
//!
//! La logique est dans `bonk_ops::job` ; ce module la présente. Avec `--progress json`, la sortie
//! standard est un flux de lignes JSON (événements `progress` puis un événement `result`), les mêmes
//! événements que la barre de la fenêtre (`repair-gpt` ne rend que le `result`) ; sinon le résultat est du texte ou, avec `--json`, un JSON.

mod args;
mod output;
mod repair;
mod verify;
mod write;

pub(crate) use args::*;
pub(crate) use output::{
    finish, schema_error, schema_plan, schema_progress, schema_repair, schema_verify, schema_write,
    schema_write_extract, target_line,
};
pub(crate) use repair::run_repair;
pub(crate) use verify::run_verify;
pub(crate) use write::run_write;

#[cfg(test)]
mod tests {
    use super::output::progress_mode;
    use super::*;

    #[test]
    fn quiet_retire_seulement_la_progression_automatique() {
        assert!(matches!(
            progress_mode(ProgressMode::Auto, true),
            ProgressMode::None
        ));
        assert!(matches!(
            progress_mode(ProgressMode::Auto, false),
            ProgressMode::Auto
        ));
        assert!(matches!(
            progress_mode(ProgressMode::Json, true),
            ProgressMode::Json
        ));
    }
}
