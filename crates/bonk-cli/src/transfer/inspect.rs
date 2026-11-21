use std::io::{self, Write};
use std::path::Path;

use bonk_ops::job::{self, ImageInspection};

use super::output::{image_line, print_json};
use super::write::{extraction_lines, loader_lines, way_line};
use crate::errors::{Code, report};

pub(crate) fn run_image_info(
    path: &Path,
    json: bool,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let inspection = match job::inspect(path) {
        Ok(inspection) => inspection,
        Err(error) => return report(err, error.code, error.message),
    };
    let written = if json {
        print_json(out, &inspection, false)
    } else {
        lines(&inspection, out)
    };
    match written {
        Ok(()) => 0,
        Err(error) => report(err, Code::OutputFailed, error),
    }
}

fn lines(inspection: &ImageInspection, out: &mut dyn Write) -> io::Result<()> {
    writeln!(out, "Image: {}", image_line(&inspection.image))?;
    writeln!(
        out,
        "  Source: {} ({})",
        inspection.source.path, inspection.source.format
    )?;
    match (inspection.route.and_then(way_line), &inspection.refusal) {
        (Some(way), _) => writeln!(out, "  Route: {way}")?,
        (None, Some(reason)) => {
            writeln!(out, "  Route: refused, {reason}")?;
            if inspection.hybrid_possible {
                writeln!(out, "  --mode hybrid writes it: starts in UEFI only")?;
            }
        }
        (None, None) => {}
    }
    match &inspection.extraction {
        Some(extraction) => extraction_lines(extraction, out),
        None => loader_lines(&inspection.efi_loaders, out),
    }
}

pub(crate) fn schema_image() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ImageInspection)).unwrap_or_default()
}
