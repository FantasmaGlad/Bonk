use crate::errors::{Code, report};
use std::io::Write;

use crate::*;

/// What a command has that is incompatible with `--fixture`: anything that would read the real system or open
/// a real disk (low level stays allowed on an image file). No default arm: a new command does not compile
/// until its answer is written here, since forgetting it would allow writing to a real disk under
/// `--fixture`.
pub(crate) fn fixture_conflict(command: &Option<Command>) -> Option<&'static str> {
    const DISK: &str = "--fixture only simulates the enumeration: writing to and raw reading of a disk are refused (use --simulate or --dry-run)";
    const GUI: &str = "the window enumerates and writes to real disks: incompatible with --fixture";
    let Some(command) = command else {
        return Some(GUI);
    };
    match command {
        Command::Gui(_) => Some(GUI),
        Command::Debug {
            action: debug::DebugAction::Layout(a),
        } => (!a.reads_image_file()).then_some(DISK),
        Command::Debug {
            action: debug::DebugAction::Hexdump(a),
        } => (!a.reads_image_file()).then_some(DISK),
        Command::Write(a)
        | Command::Disk {
            action: DiskAction::Write(a),
        } if a.touches_real_disk() => Some(DISK),
        Command::Verify(a)
        | Command::Disk {
            action: DiskAction::Verify(a),
        } if a.touches_real_disk() => Some(DISK),
        Command::RepairGpt(a)
        | Command::Disk {
            action: DiskAction::RepairGpt(a),
        } if a.touches_real_disk() => Some(DISK),
        Command::Format(a) if a.touches_real_disk() => Some(DISK),
        Command::Vault { action } if action.touches_real_disk() => Some(DISK),
        Command::ElevatedHelper { .. } => Some(DISK),
        Command::Debug {
            action: debug::DebugAction::Sources(_),
        } => Some("debug sources reads the real system: incompatible with --fixture"),
        Command::Diagnose(a) if a.capture() => {
            Some("--capture reads the real system: incompatible with --fixture")
        }
        // The service writes, formats and repairs the disks it enumerates: it would judge the captured ones
        // and open real disks of the same name.
        Command::Serve => {
            Some("bonk serve writes to the disks it lists: incompatible with --fixture")
        }
        Command::Format(_)
        | Command::Vault { .. }
        | Command::Capabilities(_)
        | Command::Write(_)
        | Command::Verify(_)
        | Command::RepairGpt(_)
        | Command::Disk {
            action:
                DiskAction::Write(_)
                | DiskAction::Verify(_)
                | DiskAction::RepairGpt(_)
                | DiskAction::List(_)
                | DiskAction::Info(_),
        }
        | Command::List(_)
        | Command::Info(_)
        | Command::Diagnose(_)
        | Command::Debug {
            action: debug::DebugAction::Filter(_),
        }
        | Command::Explain(_)
        | Command::Config { .. }
        | Command::Cache { .. }
        | Command::Catalog { .. }
        | Command::Schema(_)
        | Command::Completions { .. } => None,
    }
}

/// A replayed capture supplies the enumeration, and no command may then reach a real disk.
pub(crate) fn fixture_reads<E: Write>(
    cli: &Cli,
    err: &mut E,
) -> Result<Option<Vec<bonk_hal::SourceRead>>, u8> {
    let reads = cli
        .global
        .path()
        .map(fixture::load)
        .transpose()
        .map_err(|erreur| report(err, Code::FixtureInvalid, erreur))?;
    if reads.is_some()
        && let Some(raison) = fixture_conflict(&cli.command)
    {
        return Err(report(err, Code::FixtureInvalid, raison));
    }
    Ok(reads)
}
