use crate::enumerate::EnumerateError;
use crate::windows_facts::WindowsFacts;

mod disks;
mod ioctl;
mod volumes;

use disks::*;
use ioctl::*;
pub(super) use ioctl::{Handle, ioctl_raw, wide};
pub(super) use volumes::read_volumes;

/// Windows does not number disks beyond this in practice; numbers may have gaps.
const MAX_DISKS: u32 = 128;

pub(super) fn collect() -> Result<WindowsFacts, EnumerateError> {
    let failed = |what: &str| {
        let what = what.to_owned();
        move |source| EnumerateError::Io { what, source }
    };
    let mut instances = disk_instances();
    let mut disks = Vec::new();
    for number in 0..MAX_DISKS {
        let instance = instances.remove(&number);
        if let Some(disk) =
            read_disk(number, instance).map_err(failed(&format!("PhysicalDrive{number}")))?
        {
            disks.push(disk);
        }
    }
    Ok(WindowsFacts {
        system_drive: std::env::var("SystemDrive").ok(),
        disks,
        volumes: read_volumes().map_err(failed("volumes"))?,
    })
}
