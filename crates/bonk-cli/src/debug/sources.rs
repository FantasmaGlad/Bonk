//! `bonk debug sources` : chaque source système lue par l'énumération.

use std::io::{self, Write};

use bonk_hal::{Disk, EnumerateError, SourceRead};
use schemars::JsonSchema;
use serde::Serialize;

use crate::errors::Code;

use super::{JSON_SCHEMA_VERSION, Outcome, SourcesArgs};

#[derive(Serialize, JsonSchema)]
pub(super) struct SourcesReport<'a> {
    schema_version: u32,
    enumeration: EnumerationSummary,
    reads: &'a [SourceRead],
}

#[derive(Serialize, JsonSchema)]
pub(super) struct EnumerationSummary {
    ok: bool,
    error: Option<String>,
    disks: usize,
}

pub(crate) fn sources(
    args: &SourcesArgs,
    result: &Result<Vec<Disk>, EnumerateError>,
    reads: &[SourceRead],
    out: &mut dyn Write,
) -> io::Result<Outcome> {
    let summary = EnumerationSummary {
        ok: result.is_ok(),
        error: result.as_ref().err().map(ToString::to_string),
        disks: result.as_ref().map_or(0, Vec::len),
    };
    if args.json {
        let report = SourcesReport {
            schema_version: JSON_SCHEMA_VERSION,
            enumeration: summary,
            reads,
        };
        serde_json::to_writer_pretty(&mut *out, &report).map_err(io::Error::from)?;
        writeln!(out)?;
    } else {
        for read in reads {
            let status = if read.ok { "ok    " } else { "ÉCHEC " };
            let size = read.bytes.map_or_else(String::new, |b| format!("{b} o"));
            writeln!(
                out,
                "[{status}] {:<4} {}  {size}  {} µs",
                read.kind, read.path, read.micros
            )?;
            if let Some(error) = &read.error {
                writeln!(out, "    {error}")?;
            }
            if let (true, Some(content)) = (args.content, &read.content) {
                for line in content.lines() {
                    writeln!(out, "    | {line}")?;
                }
            }
        }
        writeln!(out, "{} lecture(s) de sources.", reads.len())?;
    }
    Ok(match result {
        Ok(_) => Outcome::Done,
        Err(erreur) => Outcome::Failed(
            Code::of_enumeration(erreur),
            format!("l'énumération a échoué : {erreur}"),
        ),
    })
}

pub(crate) fn schema_sources() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(SourcesReport)).unwrap_or_default()
}
