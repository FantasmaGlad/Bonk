use std::error::Error;
use std::fs;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use super::mounts::unescape_octal;
use super::*;
use crate::safety::{Eligibility, Exclusion, FilterOptions, evaluate};

mod filter;
mod sources;

struct Fake {
    base: PathBuf,
}

impl Fake {
    fn new() -> Result<Self, Box<dyn Error>> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, AtomicOrdering::Relaxed);
        let base = std::env::temp_dir().join(format!("bonk-enum-{}-{n}", std::process::id()));
        let fake = Self { base };
        fake.put("proc/self/mountinfo", "")?;
        fake.put(
            "proc/swaps",
            "Filename\t\t\t\tType\t\tSize\t\tUsed\t\tPriority\n",
        )?;
        fs::create_dir_all(fake.base.join("run/udev/data"))?;
        fs::create_dir_all(fake.base.join("sys/block"))?;
        fs::create_dir_all(fake.base.join("dev"))?;
        Ok(fake)
    }

    fn roots(&self) -> LinuxRoots {
        LinuxRoots::under(&self.base)
    }

    fn put(&self, rel: &str, content: &str) -> Result<(), Box<dyn Error>> {
        let path = self.base.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, content)?;
        Ok(())
    }

    fn disk(
        &self,
        name: &str,
        device_path: &str,
        devnum: &str,
        sectors: u64,
        extra: &[(&str, &str)],
    ) -> Result<(), Box<dyn Error>> {
        let dir = format!("sys/{device_path}");
        self.put(&format!("{dir}/dev"), &format!("{devnum}\n"))?;
        self.put(&format!("{dir}/size"), &format!("{sectors}\n"))?;
        self.put(&format!("{dir}/removable"), "0\n")?;
        fs::create_dir_all(self.base.join(&dir).join("holders"))?;
        for (file, content) in extra {
            self.put(&format!("{dir}/{file}"), content)?;
        }
        symlink(
            format!("../{device_path}"),
            self.base.join("sys/block").join(name),
        )?;
        Ok(())
    }

    fn partition(
        &self,
        device_path: &str,
        name: &str,
        number: u32,
        devnum: &str,
        sectors: u64,
    ) -> Result<(), Box<dyn Error>> {
        let dir = format!("sys/{device_path}/{name}");
        self.put(&format!("{dir}/dev"), &format!("{devnum}\n"))?;
        self.put(&format!("{dir}/size"), &format!("{sectors}\n"))?;
        self.put(&format!("{dir}/partition"), &format!("{number}\n"))?;
        fs::create_dir_all(self.base.join(&dir).join("holders"))?;
        Ok(())
    }

    fn udev(&self, devnum: &str, lines: &str) -> Result<(), Box<dyn Error>> {
        self.put(&format!("run/udev/data/b{devnum}"), lines)
    }

    fn mountinfo(&self, text: &str) -> Result<(), Box<dyn Error>> {
        self.put("proc/self/mountinfo", text)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.base);
    }
}

const NVME_PATH: &str = "devices/pci0000:00/0000:00:02.4/0000:02:00.0/nvme/nvme0/nvme0n1";

const USB_PATH: &str =
    "devices/pci0000:00/0000:00:14.0/usb1/1-1/1-1:1.0/host6/target6:0:0/6:0:0:0/block/sdb";
const SATA_PATH: &str = "devices/pci0000:00/0000:00:17.0/ata1/host0/target0:0:0/0:0:0:0/block/sda";

/// NVMe system disk, as read on a real machine.
fn desktop(fake: &Fake) -> Result<(), Box<dyn Error>> {
    fake.disk(
        "nvme0n1",
        NVME_PATH,
        "259:0",
        1_000_215_216,
        &[("device/model", "SAMSUNG MZVL2512HDJD-00BH1       \n")],
    )?;
    fake.partition(NVME_PATH, "nvme0n1p1", 1, "259:1", 2_097_152)?;
    fake.partition(NVME_PATH, "nvme0n1p2", 2, "259:2", 997_000_000)?;
    fake.udev(
        "259:0",
        "S:disk/by-id/nvme-eui.002538bc31bf297e\nS:disk/by-id/nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4\nS:disk/by-id/nvme-SAMSUNG_MZVL2512HDJD-00BH1_S6Z4_1\nE:ID_SERIAL_SHORT=S6Z4\nE:ID_MODEL=SAMSUNG MZVL2512HDJD-00BH1\n",
    )?;
    fake.udev(
        "259:1",
        "E:ID_FS_TYPE=vfat\nE:ID_PART_ENTRY_TYPE=c12a7328-f81f-11d2-ba4b-00a0c93ec93b\n",
    )?;
    fake.udev("259:2", "E:ID_FS_TYPE=ext4\n")?;
    fake.mountinfo(
        "44 1 259:2 / / rw,noatime shared:1 - ext4 /dev/nvme0n1p2 rw,commit=60\n\
         64 44 259:1 / /boot/efi rw,relatime shared:134 - vfat /dev/nvme0n1p1 rw,fmask=0022\n\
         34 44 0:29 / /run rw,nosuid,nodev shared:13 - tmpfs tmpfs rw,size=3017756k\n",
    )
}

fn usb_stick(fake: &Fake) -> Result<(), Box<dyn Error>> {
    fake.disk(
        "sdb",
        USB_PATH,
        "8:16",
        31_260_672,
        &[
            ("removable", "1\n"),
            ("queue/logical_block_size", "512\n"),
            ("device/vendor", "SanDisk \n"),
            ("device/model", "Cruzer Blade    \n"),
        ],
    )?;
    fake.partition(USB_PATH, "sdb1", 1, "8:17", 31_258_624)?;
    fake.udev(
        "8:16",
        "S:disk/by-id/usb-SanDisk_Cruzer_Blade_4C530001-0:0\nE:ID_SERIAL_SHORT=4C530001\nE:ID_BUS=usb\nE:ID_PATH=pci-0000:00:14.0-usb-0:2:1.0-scsi-0:0:0:0\n",
    )?;
    fake.udev("8:17", "E:ID_FS_TYPE=vfat\nE:ID_FS_LABEL=CLE\n")
}

fn verdict(disk: &Disk) -> Eligibility {
    evaluate(disk, FilterOptions::default())
}

fn find<'a>(disks: &'a [Disk], path: &str) -> Result<&'a Disk, Box<dyn Error>> {
    disks
        .iter()
        .find(|d| d.path == path)
        .ok_or_else(|| format!("disque {path} absent de {disks:?}").into())
}
