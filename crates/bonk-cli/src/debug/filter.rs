//! `bonk debug filter` : le filtre de sécurité rejoué règle par règle.

use std::io::{self, Write};

use bonk_hal::{Disk, FilterOptions, evaluate_traced, state_label};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;
use crate::info;

use super::{FilterArgs, Outcome};

/// `debug filter` est passé à 2 en retirant le champ `cdc` de chaque règle.
pub(super) const FILTER_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize, JsonSchema)]
pub(super) struct FilterReport<'a> {
    schema_version: u32,
    disk: &'a str,
    options: FilterOptions,
    verdict: &'a bonk_hal::Eligibility,
    rules: &'a [bonk_hal::RuleTrace],
}

/// Le rapport `debug filter`, en JSON, pour le service ; `None` si le disque est inconnu.
pub(crate) fn filter_value(
    target: &str,
    options: FilterOptions,
    disks: &[Disk],
) -> Option<Result<serde_json::Value, serde_json::Error>> {
    let disk = info::find(disks, target)?;
    let (verdict, rules) = evaluate_traced(disk, options);
    Some(serde_json::to_value(FilterReport {
        schema_version: FILTER_SCHEMA_VERSION,
        disk: &disk.id,
        options,
        verdict: &verdict,
        rules: &rules,
    }))
}

pub(crate) fn filter(
    args: &FilterArgs,
    disks: &[Disk],
    out: &mut dyn Write,
) -> io::Result<Outcome> {
    let Some(disk) = info::find(disks, &args.target) else {
        return Ok(Outcome::Failed(
            Code::DiskNotFound,
            format!(
                "disque introuvable : « {} » (identifiants : bonk list --all)",
                args.target
            ),
        ));
    };
    let options = FilterOptions {
        show_internal: args.show_internal,
        allow_virtual: args.allow_virtual,
    };
    let (verdict, rules) = evaluate_traced(disk, options);
    if args.json {
        let report = FilterReport {
            schema_version: FILTER_SCHEMA_VERSION,
            disk: &disk.id,
            options,
            verdict: &verdict,
            rules: &rules,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)?;
        return Ok(Outcome::Done);
    }
    writeln!(out, "Disque {} ({})", disk.id, disk.path)?;
    writeln!(
        out,
        "Réglages : disques internes {}, disques virtuels {}",
        if options.show_internal {
            "proposés"
        } else {
            "cachés"
        },
        if options.allow_virtual {
            "autorisés"
        } else {
            "interdits"
        },
    )?;
    let subject_width = rules
        .iter()
        .map(|r| r.subject.chars().count())
        .max()
        .unwrap_or(0);
    let rule_width = rules.iter().map(|r| r.rule.len()).max().unwrap_or(0);
    for rule in &rules {
        // La règle du bus ne refuse rien : quand elle « s'applique », elle décide de la proposition.
        let mark = match (rule.rule, rule.fired) {
            ("bus", true) => "VERDICT  ",
            (_, true) => "APPLIQUÉE",
            _ => "non      ",
        };
        writeln!(
            out,
            "[{mark}] {:<rule_width$}  {:<subject_width$}  {}",
            rule.rule, rule.subject, rule.facts
        )?;
    }
    writeln!(out, "Verdict : {}", state_label(&verdict))?;
    Ok(Outcome::Done)
}

pub(crate) fn schema_filter() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(FilterReport)).unwrap_or_default()
}
