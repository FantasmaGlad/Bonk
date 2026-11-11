use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Mutex;

use bonk_catalog::manifest::{self, Grub};
use bonk_core::format::table::TableKind;
use bonk_core::vault::{self as core_vault, BootFile};
use bonk_core::{BlockDevice, BlockError, partition};
use bonk_hal::{Disk, Elevation, FilterOptions};
use bonk_ops::errors::Code;
use bonk_ops::job::{
    self, Deps, GrubInfo, JobError, Target, VaultAddJob, VaultInitJob, VaultListJob, VaultUpdateJob,
};
use bonk_ui::event::{Event, Outcome};

use crate::screen::{self, MODEL, lock};
use crate::writer::{android_io, finished, push, report, same_drive};

/// GRUB as the last verified catalog names it, with the address the catalog was read from.
static GRUB: Mutex<Option<(String, Grub)>> = Mutex::new(None);

pub(crate) fn set_grub(base: &str, grub: Option<Grub>) {
    *lock(&GRUB) = grub.map(|grub| (base.to_owned(), grub));
}

/// One GRUB image to download: its processor, address and signed size.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Download {
    pub(crate) arch: String,
    pub(crate) url: String,
    pub(crate) size_bytes: u64,
}

/// What Kotlin downloads, one file per processor Bonk can write: empty while the catalog offers none.
pub(crate) fn grub_downloads() -> Vec<Download> {
    downloads(lock(&GRUB).as_ref())
}

fn downloads(offered: Option<&(String, Grub)>) -> Vec<Download> {
    let Some((base, entry)) = offered else {
        return Vec::new();
    };
    writable(entry)
        .map(|(arch, asset)| Download {
            arch: arch.clone(),
            url: format!("{base}{}", asset.path),
            size_bytes: asset.size_bytes,
        })
        .collect()
}

/// A processor this Bonk has no removable-media name for is left to a later version.
fn writable(entry: &Grub) -> impl Iterator<Item = (&String, &manifest::Asset)> {
    entry
        .efi
        .iter()
        .filter(|(arch, _)| core_vault::efi_file_name(arch).is_some())
}

fn refused(message: impl std::fmt::Display) -> JobError {
    JobError {
        code: Code::CatalogRefused,
        message: message.to_string(),
    }
}

/// The files Kotlin downloaded into `folder`, one named by each processor, accepted only with the hashes
/// the signed catalog gives.
fn boot(entry: Option<&Grub>, folder: &Path) -> Result<(Vec<BootFile>, GrubInfo), JobError> {
    let entry =
        entry.ok_or_else(|| refused("the catalog does not offer GRUB yet: try again later"))?;
    let mut images = Vec::new();
    let mut efi = BTreeMap::new();
    for (arch, asset) in writable(entry) {
        let data = std::fs::read(folder.join(arch))
            .map_err(|e| refused(format!("GRUB {} {arch}: {e}", entry.version)))?;
        if !manifest::asset_matches(asset, &data) {
            return Err(refused(format!(
                "GRUB {} {arch} does not match its signed hash",
                entry.version
            )));
        }
        efi.insert(arch.clone(), asset.sha256.clone());
        images.push((arch.as_str(), data));
    }
    if images.is_empty() {
        return Err(refused(
            "the catalog offers no GRUB image this version of Bonk can write",
        ));
    }
    let files = core_vault::boot_files(&images).map_err(refused)?;
    let info = GrubInfo {
        version: entry.version.clone(),
        build: entry.build.clone(),
        efi,
    };
    Ok((files, info))
}

/// Android says nothing of a new table: the chosen disk's partitions are read on the drive itself, as the
/// probe does, so the engine sees the vault it just wrote.
fn as_on_drive(
    disks: &[Disk],
    id: &str,
    open: &dyn Fn() -> Result<Box<dyn BlockDevice>, BlockError>,
) -> Vec<Disk> {
    let mut disks = disks.to_vec();
    if let Some(disk) = disks.iter_mut().find(|disk| disk.id == id)
        && let Ok(device) = open()
        && let Ok(layout) = partition::inspect(&*device)
    {
        disk.partitions = screen::partitions(&layout);
    }
    disks
}

pub(crate) struct Request<'a> {
    pub(crate) action: &'a str,
    pub(crate) disk_id: &'a str,
    /// What the app really opened: USB device name and its serial (empty without one).
    pub(crate) device: &'a str,
    pub(crate) serial: &'a str,
    /// `add`: the ISO and the name it gets on the vault.
    pub(crate) iso: &'a str,
    pub(crate) name: &'a str,
    /// `init` and `update`: the folder of the GRUB images Kotlin downloaded.
    pub(crate) grub: &'a str,
}

/// Runs a vault action end to end. Blocks: the app calls it off the UI thread and reads progress elsewhere.
pub(crate) fn run(
    request: &Request<'_>,
    open: impl Fn() -> Result<Box<dyn BlockDevice>, BlockError> + 'static,
) {
    let disks: Vec<Disk> = lock(&MODEL).disks.clone();
    let grub = lock(&GRUB).clone().map(|(_, grub)| grub);
    push(outcome(disks, grub.as_ref(), request, open));
}

fn outcome(
    disks: Vec<Disk>,
    grub: Option<&Grub>,
    request: &Request<'_>,
    open: impl Fn() -> Result<Box<dyn BlockDevice>, BlockError> + 'static,
) -> Event {
    if let Err(message) = same_drive(&disks, request.disk_id, request.device, request.serial) {
        return Event::Finished(Outcome::Failed {
            code: Some(Code::IdentityChanged.id().to_owned()),
            message,
        });
    }
    let open = Rc::new(open);
    let io = android_io({
        let open = Rc::clone(&open);
        move || open()
    });
    let id = request.disk_id.to_owned();
    let enumerate = move || Ok(as_on_drive(&disks, &id, &*open));
    let deps = Deps {
        enumerate: &enumerate,
        io: &io,
    };
    let target = Target::Disk(request.disk_id.to_owned());
    let options = FilterOptions::default();
    let elevation = Elevation::Never;
    match request.action {
        "list" => finished(job::vault_list(
            &VaultListJob {
                target,
                options,
                trace_io: None,
                elevation,
            },
            &deps,
        )),
        "init" => finished(
            boot(grub, Path::new(request.grub)).and_then(|(boot, grub)| {
                job::vault_init(
                    &VaultInitJob {
                        target,
                        boot,
                        grub: Some(grub),
                        table: TableKind::VAULT_DEFAULT,
                        options,
                        trace_io: None,
                        dry_run: false,
                        yes: true,
                        force: false,
                        elevation,
                    },
                    &deps,
                )
            }),
        ),
        "add" => finished(job::vault_add(
            &VaultAddJob {
                target,
                source: request.iso.into(),
                name: (!request.name.is_empty()).then(|| request.name.to_owned()),
                options,
                trace_io: None,
                yes: true,
                elevation,
            },
            &deps,
            &mut report,
        )),
        "update" => finished(
            boot(grub, Path::new(request.grub)).and_then(|(boot, grub)| {
                job::vault_update(
                    &VaultUpdateJob {
                        target,
                        boot,
                        grub: Some(grub),
                        options,
                        trace_io: None,
                        dry_run: false,
                        yes: true,
                        elevation,
                    },
                    &deps,
                )
            }),
        ),
        other => Event::Finished(Outcome::Failed {
            code: Some(Code::InvalidArgument.id().to_owned()),
            message: format!("unknown vault action: {other}"),
        }),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use bonk_catalog::manifest::Asset;
    use bonk_core::MemoryDevice;
    use bonk_hal::{Bus, DiskKind};
    use bonk_ui::event::VaultDone;

    use super::*;

    const GRUB_BYTES: &[u8] = b"MZ grub de test";
    const ARM_BYTES: &[u8] = b"MZ grub arm64 de test";

    /// One drive behind every opening, as the key behind each descriptor copy.
    struct Shared(Rc<RefCell<MemoryDevice>>);

    impl BlockDevice for Shared {
        fn size(&self) -> u64 {
            self.0.borrow().size()
        }
        fn block_size(&self) -> u32 {
            self.0.borrow().block_size()
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
            self.0.borrow().read_at(offset, buf)
        }
        fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
            self.0.borrow_mut().write_at(offset, data)
        }
        fn flush(&mut self) -> Result<(), BlockError> {
            Ok(())
        }
    }

    fn asset(path: &str, bytes: &[u8]) -> Asset {
        Asset {
            path: path.to_owned(),
            sha256: bonk_core::write::hex(&sha256(bytes)),
            size_bytes: bytes.len() as u64,
        }
    }

    fn sha256(bytes: &[u8]) -> Vec<u8> {
        use sha2::Digest;
        sha2::Sha256::digest(bytes).to_vec()
    }

    fn key(size: u64) -> Disk {
        Disk {
            id: "usb-S1-001-002".to_owned(),
            path: "/dev/bus/usb/001/002".to_owned(),
            model: "UDisk".to_owned(),
            serial: Some("S1".to_owned()),
            size_bytes: size,
            bus: Bus::Usb,
            kind: DiskKind::Physical,
            removable_flag: true,
            logical_block_size: None,
            location: Some("001-002".to_owned()),
            fs_type: None,
            holders: Vec::new(),
            partitions: Vec::new(),
        }
    }

    fn request<'a>(action: &'a str, iso: &'a str, name: &'a str, grub: &'a str) -> Request<'a> {
        Request {
            action,
            disk_id: "usb-S1-001-002",
            device: "/dev/bus/usb/001/002",
            serial: "S1",
            iso,
            name,
            grub,
        }
    }

    fn failed_code(event: &Event) -> Option<&str> {
        match event {
            Event::Finished(Outcome::Failed { code, .. }) => code.as_deref(),
            _ => None,
        }
    }

    /// Prepare, add an ISO read through a nameless descriptor path, list, update: the phone's whole vault.
    #[test]
    fn le_telephone_prepare_remplit_et_met_a_jour_un_coffre() {
        let size = 256 << 20;
        let drive = Rc::new(RefCell::new(MemoryDevice::new(size, 512).expect("disque")));
        let disks = vec![key(size)];
        let mut offered: Option<(String, Grub)> = None;
        let run = |offered: &Option<(String, Grub)>, request: &Request<'_>| {
            let drive = Rc::clone(&drive);
            let grub = offered.as_ref().map(|(_, grub)| grub);
            outcome(disks.clone(), grub, request, move || {
                Ok(Box::new(Shared(Rc::clone(&drive))) as Box<dyn BlockDevice>)
            })
        };
        let dir = std::env::temp_dir().join(format!("bonk-android-coffre-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dossier");
        // As Kotlin lays them out: one file per processor, named by it.
        let grub = dir.join("grub");
        std::fs::create_dir_all(&grub).expect("dossier");
        std::fs::write(grub.join("x86_64"), GRUB_BYTES).expect("GRUB");
        std::fs::write(grub.join("aarch64"), ARM_BYTES).expect("GRUB");
        let grub = grub.display().to_string();
        let forged = dir.join("forged");
        std::fs::create_dir_all(&forged).expect("dossier");
        std::fs::write(forged.join("x86_64"), GRUB_BYTES).expect("GRUB");
        std::fs::write(forged.join("aarch64"), b"MZ autre GRUB").expect("GRUB");
        let forged = forged.display().to_string();
        let iso = dir.join("42");
        let image = bonk_core::iso9660::testing::image(
            "ARCH_202611",
            &[
                ("arch/boot/x86_64/vmlinuz-linux", 1),
                ("arch/boot/x86_64/initramfs-linux.img", 1),
            ],
        );
        std::fs::write(&iso, &image).expect("ISO");
        let iso = iso.display().to_string();

        assert_eq!(downloads(offered.as_ref()), []);
        assert_eq!(
            failed_code(&run(&offered, &request("init", "", "", &grub))),
            Some(Code::CatalogRefused.id())
        );
        let fake = asset("grub/x.tar.xz", b"x");
        let mut efi = BTreeMap::new();
        efi.insert(
            "x86_64".to_owned(),
            asset("grub/2.14/BOOTX64.EFI", GRUB_BYTES),
        );
        efi.insert(
            "aarch64".to_owned(),
            asset("grub/2.14/BOOTAA64.EFI", ARM_BYTES),
        );
        // A processor Bonk cannot name yet: nothing to download for it.
        efi.insert("mips64".to_owned(), asset("grub/2.14/X.EFI", b"x"));
        offered = Some((
            "https://catalog.test/v1/".to_owned(),
            Grub {
                version: "2.14".to_owned(),
                released: "2026-01-14".to_owned(),
                build: "6e330c761ee82877".to_owned(),
                efi,
                source: fake.clone(),
                source_signature: fake,
            },
        ));
        assert_eq!(
            downloads(offered.as_ref()),
            [
                Download {
                    arch: "aarch64".to_owned(),
                    url: "https://catalog.test/v1/grub/2.14/BOOTAA64.EFI".to_owned(),
                    size_bytes: ARM_BYTES.len() as u64,
                },
                Download {
                    arch: "x86_64".to_owned(),
                    url: "https://catalog.test/v1/grub/2.14/BOOTX64.EFI".to_owned(),
                    size_bytes: GRUB_BYTES.len() as u64,
                },
            ]
        );

        assert_eq!(
            failed_code(&run(&offered, &request("list", "", "", ""))),
            Some(Code::NotAVault.id())
        );
        // A GRUB that is not the catalog's is refused before the drive is touched.
        assert_eq!(
            failed_code(&run(&offered, &request("init", "", "", &forged))),
            Some(Code::CatalogRefused.id())
        );
        assert!(drive.borrow().as_bytes().iter().all(|&byte| byte == 0));

        // The new partitions are read on the drive: the engine does not wait for the system to show them.
        let started = Instant::now();
        assert_eq!(
            run(&offered, &request("init", "", "", &grub)),
            Event::Finished(Outcome::Vault(VaultDone::Prepared))
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );

        assert_eq!(
            run(&offered, &request("add", &iso, "archlinux-x86_64.iso", "")),
            Event::Finished(Outcome::Vault(VaultDone::Added {
                name: "archlinux-x86_64.iso".to_owned(),
                bytes: image.len() as u64,
            }))
        );
        assert_eq!(
            run(&offered, &request("list", "", "", "")),
            Event::Finished(Outcome::Vault(VaultDone::Listed(vec![(
                "archlinux-x86_64.iso".to_owned(),
                image.len() as u64
            )])))
        );
        assert_eq!(
            run(&offered, &request("update", "", "", &grub)),
            Event::Finished(Outcome::Vault(VaultDone::Updated { changed: false }))
        );

        // Another key at the chosen place: nothing runs.
        let mut other = request("list", "", "", "");
        other.serial = "S2";
        assert_eq!(
            failed_code(&run(&offered, &other)),
            Some(Code::IdentityChanged.id())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
