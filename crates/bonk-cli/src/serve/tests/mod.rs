#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{self, Cursor, Read};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use bonk_core::{BlockDevice, BlockError, MemoryDevice};
use bonk_hal::{Bus, DiskKind, FilterOptions, Partition};

use super::*;
use crate::list;

mod disks;
mod operations;
mod protocol;

const DISK_SIZE: usize = 8 << 20;

/// Shared between threads; `slow` slows writes so an operation is still running when a cancel is sent.
#[derive(Clone)]
struct Shared {
    memory: Arc<Mutex<MemoryDevice>>,
    slow: bool,
}

impl BlockDevice for Shared {
    fn size(&self) -> u64 {
        self.memory.lock().unwrap().size()
    }
    fn block_size(&self) -> u32 {
        512
    }
    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<(), BlockError> {
        self.memory.lock().unwrap().read_at(offset, buf)
    }
    fn write_at(&mut self, offset: u64, data: &[u8]) -> Result<(), BlockError> {
        if self.slow {
            std::thread::sleep(Duration::from_millis(60));
        }
        self.memory.lock().unwrap().write_at(offset, data)
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

fn usb(id: &str) -> Disk {
    Disk {
        id: id.to_owned(),
        path: format!("/dev/{id}"),
        model: "Cle".to_owned(),
        serial: Some("S1".to_owned()),
        size_bytes: DISK_SIZE as u64,
        bus: Bus::Usb,
        kind: DiskKind::Physical,
        removable_flag: true,
        logical_block_size: None,
        location: None,
        fs_type: None,
        holders: vec![],
        partitions: vec![partition("/dev/x1", &[], false)],
    }
}

fn system() -> Disk {
    Disk {
        bus: Bus::Nvme,
        partitions: vec![partition("/dev/nvme0n1p2", &["/"], true)],
        ..usb("nvme-sys")
    }
}

struct Fake {
    device: Shared,
    cfg: Config,
}

fn fake(slow: bool, enumerate: EnumerateFn) -> Fake {
    let device = Shared {
        memory: Arc::new(Mutex::new(
            MemoryDevice::new(DISK_SIZE as u64, 512).unwrap(),
        )),
        slow,
    };
    let dev = device.clone();
    Fake {
        device,
        cfg: Config {
            enumerate,
            make_io: Arc::new(move |cancel| {
                let dev = dev.clone();
                Io {
                    prepare: Box::new(|_, _, _| Ok(())),
                    unmount: Box::new(|_, _| Ok(vec![])),
                    capabilities: Box::new(Vec::new),
                    format_partition: Box::new(|_, _, _, _| Ok(())),
                    rescan: Box::new(|_, _| Ok(())),
                    pause: Box::new(|| {}),
                    open_node: Box::new(move |_, _, _| {
                        Ok(Box::new(dev.clone()) as Box<dyn BlockDevice>)
                    }),
                    cancelled: Box::new(move || cancel.load(Ordering::SeqCst)),
                }
            }),
            config_path: None,
            vars: vec![],
            view: ViewDefaults::default(),
            interrupted: Arc::new(|| false),
            catalog: crate::catalog::Access::default(),
        },
    }
}

fn fixed() -> EnumerateFn {
    Arc::new(|| Ok(vec![usb("usb-a"), system()]))
}

#[derive(Clone)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Waits before closing: gives asynchronous events time.
struct SlowEof {
    inner: Cursor<Vec<u8>>,
    wait: Duration,
}

impl Read for SlowEof {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            std::thread::sleep(self.wait);
        }
        Ok(n)
    }
}

impl BufRead for SlowEof {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        let empty = self.inner.fill_buf()?.is_empty();
        if empty {
            std::thread::sleep(self.wait);
            return Ok(&[]);
        }
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
    }
}

/// Closes only once an expected message is out: a long operation has time to finish, without a fixed delay to
/// guess.
struct UntilSeen {
    inner: Cursor<Vec<u8>>,
    output: Buffer,
    needle: String,
}

impl Read for UntilSeen {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl BufRead for UntilSeen {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.inner.fill_buf()?.is_empty() {
            let limit = std::time::Instant::now() + Duration::from_secs(20);
            while std::time::Instant::now() < limit {
                let text = String::from_utf8_lossy(&self.output.0.lock().unwrap()).into_owned();
                if text.contains(&self.needle) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            return Ok(&[]);
        }
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
    }
}

/// Waits for `needle` in the output before closing the input.
fn session_until(requests: &[&str], cfg: &Config, needle: &str) -> Vec<Value> {
    let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
    let input = UntilSeen {
        inner: Cursor::new((requests.join("\n") + "\n").into_bytes()),
        output: buffer.clone(),
        needle: needle.to_owned(),
    };
    assert_eq!(serve(input, Box::new(buffer.clone()), cfg), 0);
    let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    text.lines()
        .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
        .collect()
}

fn session(requests: &[&str], cfg: &Config, wait: Duration) -> Vec<Value> {
    let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
    let input = SlowEof {
        inner: Cursor::new((requests.join("\n") + "\n").into_bytes()),
        wait,
    };
    let code = serve(input, Box::new(buffer.clone()), cfg);
    assert_eq!(code, 0);
    let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    text.lines()
        .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
        .collect()
}

fn quick_session(requests: &[&str]) -> Vec<Value> {
    session(requests, &fake(false, fixed()).cfg, Duration::ZERO)
}

fn response(messages: &[Value], id: u64) -> &Value {
    messages
        .iter()
        .find(|m| m["id"] == id)
        .expect("réponse attendue")
}

struct TempImage(PathBuf);

impl TempImage {
    fn new(label: &str, len: usize) -> Self {
        let path =
            std::env::temp_dir().join(format!("bonk-serve-{}-{label}.img", std::process::id()));
        std::fs::write(
            &path,
            (0..len).map(|i| (i * 11 % 251) as u8).collect::<Vec<u8>>(),
        )
        .unwrap();
        Self(path)
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
