use bonk_core::analysis::ImageReport;
use bonk_core::efi::Loader;

use crate::job::extract::Extraction;
use crate::job::target::Chosen;
use crate::job::types::{Plan, SCHEMA_VERSION, SourceInfo, WriteOutcome};

use super::*;

pub(in crate::job) fn plan(
    chosen: Chosen,
    source: SourceInfo,
    image: ImageReport,
    sha256_checked: bool,
    steps: Vec<&'static str>,
    extraction: Option<Extraction>,
    efi_loaders: Vec<Loader>,
) -> WriteOutcome {
    WriteOutcome::Plan(Box::new(Plan {
        schema_version: SCHEMA_VERSION,
        event: "plan",
        dry_run: true,
        target: chosen.info,
        source,
        image,
        sha256_checked,
        unmount: chosen.mounts,
        steps,
        extraction,
        efi_loaders,
    }))
}

pub(in crate::job) fn planned_steps(job: &WriteJob, chosen: &Chosen) -> Vec<&'static str> {
    let mut steps = Vec::new();
    if !chosen.mounts.is_empty() {
        steps.push("unmount");
    }
    steps.extend(["write", "flush"]);
    if job.verify {
        steps.push("verify");
    }
    if job.mode == WriteMode::Hybrid {
        steps.push("write_hybrid_mbr");
    }
    if job.repair_gpt {
        steps.push("repair_gpt_if_needed");
    }
    if chosen.disk.is_some() {
        steps.push("reread_table");
    }
    steps
}
