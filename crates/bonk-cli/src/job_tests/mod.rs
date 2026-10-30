//! Les opérations d'écriture sur un « disque » : tout le chemin réel (filtre, confirmation,
//! démontage, revérification d'identité, ouverture, écriture, vérification) avec des dépendances
//! injectées. Aucun vrai disque n'est ouvert : le périphérique est un tampon en mémoire.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::cell::{Cell, RefCell};
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use bonk_core::{BlockDevice, BlockError, MemoryDevice};
use bonk_hal::platform::{Capability, Missing};
use bonk_hal::{Bus, Disk, DiskKind, Elevation, OpenTargetError, Partition};

use crate::errors::{
    EXIT_CANCELLED, EXIT_SOURCE, EXIT_TARGET_REFUSED, EXIT_USAGE, EXIT_VERIFY_FAILED,
};
use crate::{Env, execute};
use bonk_ops::job::Io;

mod format;
mod internal;
mod write;

const DISK_SIZE: usize = 64 << 20;

#[derive(Clone)]
struct Shared(Rc<RefCell<MemoryDevice>>);

impl Shared {
    fn new() -> Self {
        Self(Rc::new(RefCell::new(
            MemoryDevice::new(DISK_SIZE as u64, 512).expect("géométrie valide"),
        )))
    }
}

impl BlockDevice for Shared {
    fn size(&self) -> u64 {
        self.0.borrow().size()
    }
    fn block_size(&self) -> u32 {
        512
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

/// Périphérique qui journalise le vidage du cache et signale toute relecture faite avant lui : sur un
/// vrai disque, une relecture de contrôle qui précède le vidage lit le cache du noyau, pas le support.
struct Logged {
    inner: Box<dyn BlockDevice>,
    log: Log,
    cache_clean: Cell<bool>,
}

impl BlockDevice for Logged {
    fn size(&self) -> u64 {
        self.inner.size()
    }
    fn block_size(&self) -> u32 {
        self.inner.block_size()
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        if !self.cache_clean.get() {
            self.log
                .borrow_mut()
                .push("lecture_avant_vidage_du_cache".to_owned());
        }
        self.inner.read_at(offset, buf)
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        self.cache_clean.set(false);
        self.inner.write_at(offset, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.inner.flush()
    }
    fn drop_cache(&mut self) -> Result<(), BlockError> {
        self.cache_clean.set(true);
        self.log.borrow_mut().push("drop_cache".to_owned());
        self.inner.drop_cache()
    }
}

/// Périphérique dont les relectures sont fausses : simule un support défectueux.
struct Faulty(Shared);

impl BlockDevice for Faulty {
    fn size(&self) -> u64 {
        self.0.size()
    }
    fn block_size(&self) -> u32 {
        512
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.0.read_at(offset, buf)?;
        if offset == 0 {
            buf[0] ^= 0xFF;
        }
        Ok(())
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        self.0.write_at(offset, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        Ok(())
    }
}

fn partition(path: &str, mounts: &[&str], system: bool) -> Partition {
    Partition {
        path: path.to_owned(),
        label: None,
        fs_type: Some("vfat".to_owned()),
        size_bytes: 1 << 20,
        mount_points: mounts.iter().map(|m| (*m).to_owned()).collect(),
        holders: vec![],
        is_system_volume: system,
        is_active_esp: false,
    }
}

fn usb(serial: &str) -> Disk {
    Disk {
        id: "usb-a".to_owned(),
        path: "/dev/sdb".to_owned(),
        model: "Cle".to_owned(),
        serial: Some(serial.to_owned()),
        size_bytes: DISK_SIZE as u64,
        bus: Bus::Usb,
        kind: DiskKind::Physical,
        removable_flag: true,
        fs_type: None,
        holders: vec![],
        partitions: vec![partition("/dev/sdb1", &["/run/media/u/A"], false)],
    }
}

fn system_disk() -> Disk {
    Disk {
        id: "nvme-sys".to_owned(),
        path: "/dev/nvme0n1".to_owned(),
        bus: Bus::Nvme,
        partitions: vec![partition("/dev/nvme0n1p2", &["/"], true)],
        ..usb("SYS")
    }
}

struct TempSource(PathBuf);

impl TempSource {
    fn new(label: &str, len: usize) -> Self {
        let path =
            std::env::temp_dir().join(format!("bonk-job-{}-{label}.img", std::process::id()));
        let mut data: Vec<u8> = (0..len).map(|i| (i * 13 % 251) as u8).collect();
        // Signature d'amorçage : l'analyse y voit une image de disque.
        if len >= 512 {
            data[446..510].fill(0);
            data[510] = 0x55;
            data[511] = 0xAA;
        }
        std::fs::write(&path, data).expect("écriture de l'image de test");
        Self(path)
    }
    fn from_bytes(label: &str, data: &[u8]) -> Self {
        let path =
            std::env::temp_dir().join(format!("bonk-job-{}-{label}.img", std::process::id()));
        std::fs::write(&path, data).expect("écriture de l'image de test");
        Self(path)
    }
    fn arg(&self) -> String {
        self.0.display().to_string()
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.0).expect("lecture de l'image de test")
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

type Log = Rc<RefCell<Vec<String>>>;

struct Scenario {
    log: Log,
    device: Shared,
    /// Énumérations successives : la n-ième appelée rend la n-ième liste (la dernière se répète).
    enumerations: Vec<Vec<Disk>>,
    prepare: Result<(), String>,
    unmount: Result<Vec<String>, String>,
    open: Result<(), io::ErrorKind>,
    faulty_reads: bool,
    cancel: bool,
    /// Annulation demandée pendant l'attente de l'autorisation.
    cancel_on_prepare: bool,
    seen_elevation: Rc<Cell<Option<Elevation>>>,
    vars: Vec<(String, String)>,
    capabilities: Vec<Capability>,
}

impl Scenario {
    fn new() -> Self {
        Self {
            log: Rc::new(RefCell::new(Vec::new())),
            device: Shared::new(),
            enumerations: vec![vec![usb("S1"), system_disk()]],
            prepare: Ok(()),
            unmount: Ok(vec!["/run/media/u/A".to_owned()]),
            open: Ok(()),
            faulty_reads: false,
            cancel: false,
            cancel_on_prepare: false,
            seen_elevation: Rc::new(Cell::new(None)),
            vars: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    fn run(&self, args: &[&str]) -> (u8, String, String) {
        let calls = Cell::new(0usize);
        let enumerations = self.enumerations.clone();
        let enumerate = move || {
            let n = calls.get();
            calls.set(n + 1);
            Ok(enumerations[n.min(enumerations.len() - 1)].clone())
        };
        let log = Rc::clone(&self.log);
        let unmount = self.unmount.clone();
        let log_unmount = Rc::clone(&self.log);
        let device = self.device.clone();
        let (open, faulty) = (self.open, self.faulty_reads);
        let cancel = self.cancel;
        let cancel_on_prepare = self.cancel_on_prepare;
        let capabilities = self.capabilities.clone();
        let log_format = Rc::clone(&self.log);
        let log_rescan = Rc::clone(&self.log);
        let cancelled = Rc::new(Cell::new(false));
        let cancel_flag = Rc::clone(&cancelled);
        let seen_elevation = Rc::clone(&self.seen_elevation);
        let log_prepare = self.log.clone();
        let prepare = self.prepare.clone();
        let io = Io {
            prepare: Box::new(move |path, _, _| {
                cancel_flag.set(cancel_flag.get() || cancel_on_prepare);
                log_prepare
                    .borrow_mut()
                    .push(format!("prepare:{}", path.display()));
                prepare
                    .clone()
                    .map_err(|m| BlockError::Io(io::Error::new(io::ErrorKind::PermissionDenied, m)))
            }),
            unmount: Box::new(move |disk: &Disk, elevation| {
                seen_elevation.set(Some(elevation));
                log_unmount
                    .borrow_mut()
                    .push(format!("unmount:{}", disk.id));
                unmount.clone().map_err(OpenTargetError::Unmount)
            }),
            open_node: Box::new(move |path, access, _| {
                log.borrow_mut()
                    .push(format!("open:{}:{access:?}", path.display()));
                match open {
                    Err(kind) => Err(BlockError::Io(io::Error::new(kind, "refusé"))),
                    Ok(()) => {
                        let inner: Box<dyn BlockDevice> = if faulty {
                            Box::new(Faulty(device.clone()))
                        } else {
                            Box::new(device.clone())
                        };
                        Ok(Box::new(Logged {
                            inner,
                            log: Rc::clone(&log),
                            cache_clean: Cell::new(false),
                        }) as Box<dyn BlockDevice>)
                    }
                }
            }),
            cancelled: Box::new(move || cancel || cancelled.get()),
            capabilities: Box::new(move || capabilities.clone()),
            format_partition: Box::new(move |path, fs, label, _| {
                log_format.borrow_mut().push(format!(
                    "format:{}:{fs}:{}",
                    path.display(),
                    label.unwrap_or("")
                ));
                Ok(())
            }),
            rescan: Box::new(move |path, _| {
                log_rescan
                    .borrow_mut()
                    .push(format!("rescan:{}", path.display()));
                Ok(())
            }),
            pause: Box::new(|| {}),
        };
        let env = Env {
            io,
            vars: self.vars.clone(),
            ..Env::default()
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            std::iter::once("bonk").chain(args.iter().copied()),
            &env,
            &mut out,
            &mut err,
            |_| Ok(()),
            enumerate,
            |_| Ok(()),
        );
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    fn log(&self) -> Vec<String> {
        self.log.borrow().clone()
    }

    fn written(&self, len: usize) -> Vec<u8> {
        self.device.0.borrow().as_bytes()[..len].to_vec()
    }
}

fn internal() -> Disk {
    Disk {
        id: "sata-x".to_owned(),
        path: "/dev/sda".to_owned(),
        bus: Bus::Sata,
        removable_flag: false,
        partitions: vec![],
        ..usb("I1")
    }
}

fn with_internal() -> Scenario {
    let mut scenario = Scenario::new();
    scenario.enumerations = vec![vec![usb("S1"), internal()]];
    scenario
}

const CONFIRMATION_REQUIRED: &str = "E2005";
