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

mod cache;
mod compressed;
mod devices;
mod download;
mod format;
mod inspect;
mod internal;
pub(crate) mod vault;
mod vault_families;
mod verify;
mod write;

use devices::*;

const DISK_SIZE: usize = 64 << 20;

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
        logical_block_size: None,
        location: None,
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
        if len >= 512 {
            data[446..510].fill(0);
            data[510] = 0x55;
            data[511] = 0xAA;
        }
        std::fs::write(&path, data).expect("écriture de l'image de test");
        Self(path)
    }
    fn dir(label: &str) -> Self {
        Self(std::env::temp_dir().join(format!("bonk-job-{}-{label}", std::process::id())))
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
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Log = Rc<RefCell<Vec<String>>>;

#[derive(Default)]
pub(crate) struct ServedCatalog {
    keys: Vec<[u8; 32]>,
    files: Rc<std::collections::HashMap<String, Vec<u8>>>,
}

impl ServedCatalog {
    pub(crate) fn access(&self) -> crate::catalog::Access {
        let (files, sized) = (Rc::clone(&self.files), Rc::clone(&self.files));
        crate::catalog::Access {
            keys: self.keys.clone(),
            fetch: Box::new(move |address| {
                files
                    .get(address)
                    .cloned()
                    .ok_or_else(|| format!("{address}: not served"))
            }),
            fetch_file: Box::new(move |address, size| match sized.get(address) {
                Some(data) if data.len() as u64 <= size => Ok(data.clone()),
                Some(_) => Err(format!("{address}: larger than its signed size")),
                None => Err(format!("{address}: not served")),
            }),
        }
    }
}

struct Scenario {
    log: Log,
    device: Shared,
    /// Successive enumerations: the n-th call returns the n-th list (the last one repeats).
    enumerations: Vec<Vec<Disk>>,
    prepare: Result<(), String>,
    unmount: Result<Vec<String>, String>,
    open: Result<(), io::ErrorKind>,
    faults: Faults,
    rescan_fails: bool,
    cancel: bool,
    cancel_on_prepare: bool,
    cancel_on_host_format: bool,
    seen_elevation: Rc<Cell<Option<Elevation>>>,
    on_first_enumeration: Option<Rc<dyn Fn()>>,
    vars: Vec<(String, String)>,
    capabilities: Vec<Capability>,
    catalog: ServedCatalog,
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
            faults: Faults::default(),
            rescan_fails: false,
            cancel: false,
            cancel_on_prepare: false,
            cancel_on_host_format: false,
            seen_elevation: Rc::new(Cell::new(None)),
            on_first_enumeration: None,
            vars: Vec::new(),
            capabilities: Vec::new(),
            catalog: ServedCatalog::default(),
        }
    }

    fn sized(size: u64) -> Self {
        let mut key = usb("S1");
        key.size_bytes = size;
        Self {
            device: Shared::sized(size),
            enumerations: vec![vec![key, system_disk()]],
            ..Self::new()
        }
    }

    fn run(&self, args: &[&str]) -> (u8, String, String) {
        let env = Env {
            io: self.io(),
            vars: self.vars.clone(),
            catalog: self.catalog.access(),
            ..Env::default()
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = execute(
            std::iter::once("bonk").chain(args.iter().copied()),
            &env,
            &mut out,
            &mut err,
            |_| Ok(()),
            self.enumerate(),
            |_| Ok(()),
        );
        (
            code,
            String::from_utf8_lossy(&out).into_owned(),
            String::from_utf8_lossy(&err).into_owned(),
        )
    }

    fn enumerate(&self) -> impl Fn() -> Result<Vec<Disk>, bonk_hal::EnumerateError> + use<> {
        let calls = Cell::new(0usize);
        let enumerations = self.enumerations.clone();
        let hook = self.on_first_enumeration.clone();
        move || {
            let n = calls.get();
            calls.set(n + 1);
            if let (0, Some(hook)) = (n, &hook) {
                hook();
            }
            Ok(enumerations[n.min(enumerations.len() - 1)].clone())
        }
    }

    fn io(&self) -> Io {
        let open_devices = Rc::new(Cell::new(0usize));
        let cancelled = Rc::new(Cell::new(false));
        let (cancel, capabilities) = (self.cancel, self.capabilities.clone());
        let cancel_flag = Rc::clone(&cancelled);
        Io {
            prepare: self.prepare_hook(Rc::clone(&cancelled)),
            unmount: self.unmount_hook(),
            open_node: self.open_hook(Rc::clone(&open_devices)),
            cancelled: Box::new(move || cancel || cancel_flag.get()),
            capabilities: Box::new(move || capabilities.clone()),
            format_partition: self.format_hook(cancelled),
            rescan: self.rescan_hook(open_devices),
            pause: Box::new(|| {}),
        }
    }

    fn prepare_hook(&self, cancelled: Rc<Cell<bool>>) -> bonk_ops::job::PrepareFn {
        let (log, prepare, cancel_on_prepare) = (
            Rc::clone(&self.log),
            self.prepare.clone(),
            self.cancel_on_prepare,
        );
        Box::new(move |path, _, _| {
            cancelled.set(cancelled.get() || cancel_on_prepare);
            log.borrow_mut().push(format!("prepare:{}", path.display()));
            prepare
                .clone()
                .map_err(|m| BlockError::Io(io::Error::new(io::ErrorKind::PermissionDenied, m)))
        })
    }

    fn unmount_hook(&self) -> bonk_ops::job::UnmountFn {
        let (log, unmount, seen) = (
            Rc::clone(&self.log),
            self.unmount.clone(),
            Rc::clone(&self.seen_elevation),
        );
        Box::new(move |disk: &Disk, elevation| {
            seen.set(Some(elevation));
            log.borrow_mut().push(format!("unmount:{}", disk.id));
            unmount.clone().map_err(OpenTargetError::Unmount)
        })
    }

    fn open_hook(&self, open_devices: Rc<Cell<usize>>) -> bonk_ops::job::OpenNodeFn {
        let (log, device, open, faults) = (
            Rc::clone(&self.log),
            self.device.clone(),
            self.open,
            self.faults,
        );
        Box::new(move |path, access, _| {
            log.borrow_mut()
                .push(format!("open:{}:{access:?}", path.display()));
            match open {
                Err(kind) => Err(BlockError::Io(io::Error::new(kind, "refusé"))),
                Ok(()) => {
                    let inner: Box<dyn BlockDevice> = Box::new(Faulty {
                        inner: device.clone(),
                        faults,
                        written: 0,
                        first_read_done: Cell::new(false),
                    });
                    open_devices.set(open_devices.get() + 1);
                    Ok(Box::new(Logged {
                        inner,
                        log: Rc::clone(&log),
                        cache_clean: Cell::new(false),
                        open_devices: Rc::clone(&open_devices),
                    }) as Box<dyn BlockDevice>)
                }
            }
        })
    }

    fn format_hook(&self, cancelled: Rc<Cell<bool>>) -> bonk_ops::job::FormatPartitionFn {
        let (log, cancel_on_host_format) = (Rc::clone(&self.log), self.cancel_on_host_format);
        Box::new(move |path, fs, label, _| {
            cancelled.set(cancelled.get() || cancel_on_host_format);
            log.borrow_mut().push(format!(
                "format:{}:{fs}:{}",
                path.display(),
                label.unwrap_or("")
            ));
            Ok(())
        })
    }

    fn rescan_hook(&self, open_devices: Rc<Cell<usize>>) -> bonk_ops::job::RescanFn {
        let (log, rescan_fails) = (Rc::clone(&self.log), self.rescan_fails);
        Box::new(move |path, _| {
            // The service cannot reread the table of a disk still opened exclusively.
            let call = match open_devices.get() {
                0 => "rescan",
                _ => "rescan_disque_encore_ouvert",
            };
            log.borrow_mut().push(format!("{call}:{}", path.display()));
            if rescan_fails {
                return Err(BlockError::Io(io::Error::other("service absent")));
            }
            Ok(())
        })
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
        logical_block_size: None,
        location: None,
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

fn serve_file(body: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    serve(body, false)
}

fn serve_ranges(body: Vec<u8>) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    serve(body, true)
}

fn serve(body: Vec<u8>, ranges: bool) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::{BufRead, BufReader, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = std::sync::Arc::clone(&hits);
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            let mut range = None;
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                range = range.or_else(|| requested_range(&line));
                line.clear();
            }
            match range.filter(|_| ranges) {
                Some((start, end)) => {
                    let last = end.min(body.len() - 1);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 206 P\r\ncontent-range: bytes {start}-{last}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len(),
                        last + 1 - start
                    );
                    let _ = stream.write_all(&body[start..=last]);
                }
                None => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(&body);
                }
            }
        }
    });
    (format!("http://127.0.0.1:{port}/image.img"), hits)
}

fn requested_range(line: &str) -> Option<(usize, usize)> {
    let lower = line.trim_end().to_ascii_lowercase();
    let (start, end) = lower.strip_prefix("range: bytes=")?.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?))
}

fn sha256_of(bytes: &[u8]) -> String {
    bonk_core::write::hash_stream(
        &mut std::io::Cursor::new(bytes),
        None,
        bonk_core::write::DEFAULT_CHUNK,
        &mut |_| bonk_core::write::Control::Continue,
    )
    .unwrap()
}
