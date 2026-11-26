use std::io::{self, Write};
use std::path::Path;
use std::sync::Arc;

use bonk_catalog::RemoteFile;
use bonk_core::read_at::ReadAt;
use bonk_core::vault::VaultFamily;
use bonk_ops::job::{self, ImageFile, ImageInspection, JobError};

use super::output::{image_line, print_json};
use super::write::{extraction_lines, loader_lines, way_line};
use crate::errors::{Code, report};

struct Remote(RemoteFile);

impl ReadAt for Remote {
    fn read_exact_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
        self.0.read_exact_at(offset, buf)
    }
}

// An address is read by range requests: only the parts the analysis needs travel.
fn image_file(target: &str) -> Result<ImageFile, JobError> {
    if !bonk_catalog::is_url(target) {
        return ImageFile::open(Path::new(target));
    }
    let remote = RemoteFile::open(target).map_err(|e| crate::fetch::download_error(&e))?;
    Ok(ImageFile::at(
        target.to_owned(),
        remote.len(),
        Arc::new(Remote(remote)),
    ))
}

pub(crate) fn run_image_info(
    target: &str,
    json: bool,
    families: &[VaultFamily],
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let inspected = image_file(target).and_then(|file| job::inspect_image(&file, families));
    let inspection = match inspected {
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
    if let Some(vault) = &inspection.vault {
        writeln!(
            out,
            "  Vault: {}, {}",
            vault.name,
            crate::vault::arch_name(vault.arch)
        )?;
    }
    match &inspection.extraction {
        Some(extraction) => extraction_lines(extraction, out),
        None => loader_lines(&inspection.efi_loaders, out),
    }
}

pub(crate) fn schema_image() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(ImageInspection)).unwrap_or_default()
}
