use std::fmt;

use schemars::JsonSchema;
use serde::Serialize;

use crate::analysis::{ImageKind, ImageReport};
use crate::hybrid::NotHybrid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteMode {
    #[default]
    Auto,
    Raw,
    Extract,
    Hybrid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route<W, M> {
    Raw,
    Hybrid(M),
    Extract(W),
}

/// How the image reaches the drive: as is, as is with an MBR naming its EFI image, which starts in UEFI only, or copied to a Windows drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Way {
    Raw,
    Hybrid,
    Extract,
}

impl<W, M> Route<W, M> {
    pub fn way(&self) -> Way {
        match self {
            Self::Raw => Way::Raw,
            Self::Hybrid(_) => Way::Hybrid,
            Self::Extract(_) => Way::Extract,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    Truncated { declared: u64 },
    NoDiskBoot { udf: bool },
    NoStructure,
    ExtractNeedsPlainIso,
    NotWindows,
    NotHybrid(NotHybrid),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { declared } => write!(f, "the image declares {declared} bytes"),
            Self::NoDiskBoot { udf } => write!(
                f,
                "ISO without disk boot: it would not boot from a USB drive{}",
                if *udf {
                    "; Bonk does not recognize a Windows installation in it"
                } else {
                    ""
                }
            ),
            Self::NoStructure => f.write_str("no boot structure or partition table recognized"),
            Self::ExtractNeedsPlainIso => {
                f.write_str("--mode extract requires an uncompressed Windows installation ISO")
            }
            Self::NotWindows => f.write_str(
                "this is not a Windows installation or PE ISO (UDF, sources/install.wim, install.esd or boot.wim, efi/boot/bootx64.efi, bootaa64.efi or bootia32.efi)",
            ),
            Self::NotHybrid(reason) => write!(f, "--mode hybrid: {reason}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned<W, M> {
    pub route: Route<W, M>,
    pub overridden: Option<Refusal>,
}

#[must_use]
pub fn looks_for_windows(report: &ImageReport, mode: WriteMode, compressed: bool) -> bool {
    !compressed
        && match mode {
            WriteMode::Extract => true,
            WriteMode::Auto => report.udf && !report.is_writable_as_is(),
            WriteMode::Raw | WriteMode::Hybrid => false,
        }
}

pub fn plan<W, M>(
    report: &ImageReport,
    mode: WriteMode,
    compressed: bool,
    windows: Option<W>,
    hybrid: impl FnOnce() -> Result<M, NotHybrid>,
) -> Result<Planned<W, M>, Refusal> {
    let chosen = |route| Planned {
        route,
        overridden: None,
    };
    match (mode, windows) {
        (WriteMode::Extract, _) if compressed => return Err(Refusal::ExtractNeedsPlainIso),
        (WriteMode::Extract | WriteMode::Auto, Some(found)) => {
            return Ok(chosen(Route::Extract(found)));
        }
        (WriteMode::Extract, None) => return Err(Refusal::NotWindows),
        _ => {}
    }
    let problem = refusal(report);
    match mode {
        WriteMode::Hybrid => {
            let mbr = hybrid().map_err(Refusal::NotHybrid)?;
            match problem {
                None | Some(Refusal::NoDiskBoot { .. }) => Ok(chosen(Route::Hybrid(mbr))),
                Some(other) => Err(other),
            }
        }
        WriteMode::Raw => Ok(Planned {
            route: Route::Raw,
            overridden: problem,
        }),
        WriteMode::Auto | WriteMode::Extract => problem.map_or(Ok(chosen(Route::Raw)), Err),
    }
}

fn refusal(report: &ImageReport) -> Option<Refusal> {
    if report.truncated == Some(true) {
        Some(Refusal::Truncated {
            declared: report.declared_bytes.unwrap_or_default(),
        })
    } else if report.is_writable_as_is() {
        None
    } else if report.kind == ImageKind::PlainIso {
        Some(Refusal::NoDiskBoot { udf: report.udf })
    } else {
        Some(Refusal::NoStructure)
    }
}

#[cfg(test)]
mod tests;
