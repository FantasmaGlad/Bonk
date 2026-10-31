#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{self, Cursor, Read};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use bonk_core::{BlockDevice, BlockError, MemoryDevice};
use bonk_hal::{Bus, DiskKind, FilterOptions, Partition};

use crate::list;

use super::*;

const DISK_SIZE: usize = 8 << 20;

/// Périphérique en mémoire partagé entre fils ; `slow` ralentit les écritures pour qu'une
/// opération reste en cours le temps d'envoyer une annulation.
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

/// Entrée qui attend avant de se fermer : laisse le temps aux événements asynchrones.
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

/// Entrée qui ne se ferme qu'une fois qu'un message attendu est sorti : une opération longue a
/// le temps de finir, sans durée fixe à deviner.
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

/// Comme [`session`], mais attend `needle` dans la sortie avant de fermer l'entrée.
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

/// Chaque nom désigne une seule méthode, et `methods` les rend toutes.
#[test]
fn chaque_methode_a_un_nom_unique() {
    let noms: std::collections::BTreeSet<_> = Method::ALL.iter().map(|m| m.name()).collect();
    assert_eq!(noms.len(), Method::ALL.len());
    for m in Method::ALL {
        assert_eq!(Method::from_name(m.name()), Some(m));
    }
    assert_eq!(Method::from_name("inexistante"), None);
}

#[test]
fn demarrage_ping_version_methodes() {
    let out = quick_session(&[
        r#"{"id":1,"method":"ping"}"#,
        r#"{"id":2,"method":"version"}"#,
        r#"{"id":3,"method":"methods"}"#,
    ]);
    assert_eq!(out[0]["event"], "ready");
    assert_eq!(out[0]["protocol"], PROTOCOL);
    assert_eq!(response(&out, 1)["result"]["pong"], true);
    assert_eq!(response(&out, 2)["result"]["protocol"], PROTOCOL);
    let methods = response(&out, 3)["result"].as_array().unwrap().clone();
    assert_eq!(methods.len(), Method::ALL.len());
    assert!(methods.iter().any(|m| m["method"] == "disk.write"));
}

#[test]
fn l_id_est_rendu_tel_quel_quel_que_soit_son_type() {
    let out = quick_session(&[
        r#"{"id":"abc","method":"ping"}"#,
        r#"{"id":{"x":[1]},"method":"ping"}"#,
        r#"{"method":"ping"}"#,
    ]);
    assert_eq!(out[1]["id"], "abc");
    assert_eq!(out[2]["id"], json!({"x": [1]}));
    assert_eq!(out[3]["id"], Value::Null);
}

#[test]
fn disk_list_meme_contenu_que_la_cli() {
    let out = quick_session(&[
        r#"{"id":1,"method":"disk.list"}"#,
        r#"{"id":2,"method":"disk.list","params":{"all":true}}"#,
    ]);
    let default = &response(&out, 1)["result"];
    assert_eq!(default["schema_version"], 1);
    assert_eq!(default["disks"].as_array().unwrap().len(), 1);
    assert_eq!(default["disks"][0]["id"], "usb-a");
    let all = &response(&out, 2)["result"];
    assert_eq!(all["disks"].as_array().unwrap().len(), 2);
    assert_eq!(all["disks"][1]["eligibility"]["verdict"], "excluded");
    let expected =
        list::report_value(FilterOptions::default(), true, &[usb("usb-a"), system()]).unwrap();
    assert_eq!(*all, expected);
}

#[test]
fn info_et_debug_filter() {
    let out = quick_session(&[
        r#"{"id":1,"method":"disk.info","params":{"target":"usb-a"}}"#,
        r#"{"id":2,"method":"disk.info","params":{"target":"absent"}}"#,
        r#"{"id":3,"method":"debug.filter","params":{"target":"nvme-sys"}}"#,
    ]);
    assert_eq!(response(&out, 1)["result"]["disk"]["id"], "usb-a");
    assert_eq!(response(&out, 2)["error"]["code"], "E1003");
    let rules = response(&out, 3)["result"]["rules"]
        .as_array()
        .unwrap()
        .clone();
    assert!(
        rules
            .iter()
            .any(|r| r["rule"] == "system_volume" && r["fired"] == true)
    );
}

#[test]
fn erreurs_de_protocole_avec_leur_code() {
    let out = quick_session(&[
        "ceci n'est pas du json",
        r#"{"id":1,"method":"inconnue"}"#,
        r#"{"id":2,"method":"disk.list","params":{"nimporte":true}}"#,
        r#"{"id":3,"method":"disk.info","params":{}}"#,
        r#"{"id":4,"method":"config.set","params":{"key":"lang"}}"#,
    ]);
    assert_eq!(out[1]["id"], Value::Null, "{out:?}");
    assert_eq!(out[1]["error"]["code"], "E6001");
    for id in 1..=4 {
        let error = &response(&out, id)["error"];
        assert_eq!(error["code"], "E6001", "{id} : {error}");
        assert_eq!(error["exit_code"], 1);
    }
    assert!(
        response(&out, 1)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("methods")
    );
}

#[test]
fn explain_schema_et_configuration() {
    let file = std::env::temp_dir().join(format!("bonk-serve-config-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&file);
    let mut fake = fake(false, fixed());
    fake.cfg.config_path = Some(file.clone());
    let out = session(
        &[
            r#"{"id":1,"method":"explain","params":{"code":"E4002"}}"#,
            r#"{"id":2,"method":"explain","params":{"code":"E0"}}"#,
            r#"{"id":3,"method":"schema","params":{"name":"write"}}"#,
            r#"{"id":4,"method":"schema"}"#,
            r#"{"id":5,"method":"config.set","params":{"key":"view.show_all","value":"oui"}}"#,
            r#"{"id":6,"method":"config.get","params":{"key":"view.show_all"}}"#,
            r#"{"id":7,"method":"config.list"}"#,
            r#"{"id":8,"method":"config.set","params":{"key":"lang","value":"en"}}"#,
            r#"{"id":9,"method":"disk.list"}"#,
        ],
        &fake.cfg,
        Duration::ZERO,
    );
    assert_eq!(response(&out, 1)["result"]["codes"][0]["exit_code"], 4);
    assert_eq!(response(&out, 2)["error"]["code"], "E0001");
    assert_eq!(response(&out, 3)["result"]["type"], "object");
    assert!(
        response(&out, 4)["result"]["schemas"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "serve-request")
    );
    assert_eq!(response(&out, 6)["result"]["output"], "true");
    assert_eq!(response(&out, 7)["result"]["settings"][3]["value"], "true");
    assert_eq!(response(&out, 8)["error"]["code"], "E5002");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn ecriture_avec_progression_puis_resultat() {
    let source = TempImage::new("ecriture", 9 << 20);
    let Fake { device, cfg } = fake(false, fixed());
    // Image de 9 Mio sur un disque de 8 Mio : trop grande ; on en prend une qui tient.
    let small = TempImage::new("petite", 6 << 20);
    let request = format!(
        r#"{{"id":7,"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{:?},"yes":true}}}}"#,
        small.0.display().to_string()
    );
    let too_big = format!(
        r#"{{"id":8,"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{:?},"yes":true}}}}"#,
        source.0.display().to_string()
    );
    let out = session_until(&[&request], &cfg, r#""id":7"#);
    let result = &response(&out, 7)["result"];
    assert_eq!(result["event"], "result", "{out:?}");
    assert_eq!(result["write"]["bytes"], 6 << 20);
    assert!(result["verify"]["bytes"] == 6 << 20);
    // L'ordre des messages : prêt, démarrage, progression (avec l'id de la requête), résultat.
    assert_eq!(out[1]["event"], "job_started");
    assert_eq!(out[1]["request_id"], 7);
    let progress: Vec<&Value> = out.iter().filter(|m| m["event"] == "progress").collect();
    assert!(progress.len() >= 4 && progress.iter().all(|p| p["request_id"] == 7));
    assert_eq!(out.last().unwrap()["id"], 7);
    let written = device.memory.lock().unwrap().as_bytes()[..6 << 20].to_vec();
    assert_eq!(written, std::fs::read(&small.0).unwrap());
    // Trop grande : refusée avec son code, sans toucher au disque.
    let refused = session(&[&too_big], &fake(false, fixed()).cfg, Duration::ZERO);
    assert_eq!(response(&refused, 8)["error"]["code"], "E3003");
}

#[test]
fn les_garde_fous_de_la_cli_s_appliquent_au_service() {
    let source = TempImage::new("garde", 1 << 20);
    let path = source.0.display().to_string();
    let sans_yes = format!(
        r#"{{"id":1,"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{path:?}}}}}"#
    );
    let interdit = format!(
        r#"{{"id":2,"method":"disk.write","params":{{"mode":"raw","target":"nvme-sys","source":{path:?},"yes":true,"show_internal":true}}}}"#
    );
    let plan = format!(
        r#"{{"id":3,"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{path:?},"dry_run":true}}}}"#
    );
    let par_chemin = format!(
        r#"{{"id":4,"method":"disk.write","params":{{"mode":"raw","target":"/dev/usb-a","source":{path:?},"yes":true}}}}"#
    );
    let sans_source =
        r#"{"id":5,"method":"disk.write","params":{"target":"usb-a","yes":true}}"#.to_owned();
    let f = fake(false, fixed());
    // Les opérations s'enchaînent : une à la fois, chacune dans sa session.
    for (id, request, code) in [
        (1, sans_yes, "E2004"),
        (2, interdit, "E2001"),
        (4, par_chemin, "E1003"),
        (5, sans_source, "E6001"),
    ] {
        let out = session(&[&request], &f.cfg, Duration::ZERO);
        assert_eq!(response(&out, id)["error"]["code"], code, "{id}");
    }
    let out = session(&[&plan], &f.cfg, Duration::ZERO);
    assert_eq!(response(&out, 3)["result"]["event"], "plan");
    // Rien n'a été écrit par aucune de ces requêtes.
    assert!(
        f.device
            .memory
            .lock()
            .unwrap()
            .as_bytes()
            .iter()
            .all(|b| *b == 0)
    );
}

#[test]
fn annulation_et_une_seule_operation_a_la_fois() {
    let source = TempImage::new("annule", 7 << 20);
    let path = source.0.display().to_string();
    let write = |id: u64| {
        format!(
            r#"{{"id":{id},"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{path:?},"yes":true,"no_verify":true}}}}"#
        )
    };
    let Fake { device, cfg } = fake(true, fixed());
    let out = session(
        &[&write(1), &write(2), r#"{"id":3,"method":"cancel"}"#],
        &cfg,
        Duration::from_millis(0),
    );
    // La seconde opération est refusée pendant que la première tourne.
    assert_eq!(response(&out, 2)["error"]["code"], "E6001");
    assert!(
        response(&out, 2)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("déjà en cours")
    );
    assert_eq!(response(&out, 3)["result"]["cancelling"], true);
    // La première est annulée proprement : code dédié, support partiel.
    let first = response(&out, 1);
    assert_eq!(first["error"]["code"], "E4003", "{first}");
    assert_eq!(first["error"]["exit_code"], 5);
    let written = device
        .memory
        .lock()
        .unwrap()
        .as_bytes()
        .iter()
        .filter(|b| **b != 0)
        .count();
    assert!(written < 7 << 20, "l'écriture aurait dû s'arrêter en route");
    // Sans opération en cours, cancel répond qu'il n'y a rien à annuler.
    let idle = quick_session(&[r#"{"id":1,"method":"cancel"}"#]);
    assert_eq!(response(&idle, 1)["result"]["cancelling"], false);
}

/// Entrée que le client garde ouverte : rien n'arrive après les requêtes. Elle se ferme d'elle-même
/// au bout de `deadline`, pour qu'un service qui ne s'arrêterait pas fasse échouer le test au lieu
/// de le bloquer.
struct KeptOpen {
    inner: Cursor<Vec<u8>>,
    deadline: std::time::Instant,
}

impl Read for KeptOpen {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 {
            while std::time::Instant::now() < self.deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(n)
    }
}

impl BufRead for KeptOpen {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.inner.fill_buf()?.is_empty() {
            while std::time::Instant::now() < self.deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        self.inner.fill_buf()
    }
    fn consume(&mut self, amt: usize) {
        self.inner.consume(amt);
    }
}

/// Ctrl+C ou SIGTERM : l'opération en cours est annulée entre deux blocs, sa réponse part, et le
/// service rend le code d'annulation sans attendre que le client ferme l'entrée.
#[test]
fn un_signal_d_arret_annule_l_operation_et_rend_la_main() {
    let source = TempImage::new("signal", 7 << 20);
    let path = source.0.display().to_string();
    let write = format!(
        r#"{{"id":1,"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{path:?},"yes":true,"no_verify":true}}}}"#
    );
    let mut f = fake(true, fixed());
    let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
    let seen = buffer.clone();
    f.cfg.interrupted =
        Arc::new(move || String::from_utf8_lossy(&seen.0.lock().unwrap()).contains("job_started"));
    let input = KeptOpen {
        inner: Cursor::new((write + "\n").into_bytes()),
        deadline: std::time::Instant::now() + Duration::from_secs(20),
    };
    let started = std::time::Instant::now();
    let code = serve(input, Box::new(buffer.clone()), &f.cfg);
    assert_eq!(code, crate::errors::EXIT_CANCELLED);
    assert!(started.elapsed() < Duration::from_secs(10));
    let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    let out: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(response(&out, 1)["error"]["code"], "E4003");
}

#[test]
fn fermer_l_entree_annule_l_operation_et_rend_la_main() {
    let source = TempImage::new("eof", 7 << 20);
    let path = source.0.display().to_string();
    let write = format!(
        r#"{{"id":1,"method":"disk.write","params":{{"mode":"raw","target":"usb-a","source":{path:?},"yes":true,"no_verify":true}}}}"#
    );
    let f = fake(true, fixed());
    let started = std::time::Instant::now();
    let out = session(&[&write], &f.cfg, Duration::ZERO);
    // La session se termine sans attendre les ~7 blocs lents, et l'opération est dite annulée.
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(response(&out, 1)["error"]["code"], "E4003");
}

#[test]
fn surveillance_des_disques() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let enumerate: EnumerateFn = Arc::new(move || {
        // La troisième énumération voit une seconde clé.
        if counter.fetch_add(1, Ordering::SeqCst) >= 2 {
            Ok(vec![usb("usb-a"), usb("usb-b")])
        } else {
            Ok(vec![usb("usb-a")])
        }
    });
    let f = fake(false, enumerate);
    // On attend le second événement au lieu de deviner une durée : sur un exécuteur chargé (macOS en CI),
    // l'intervalle de 100 ms s'allonge et une attente fixe de 700 ms ne voyait que l'événement initial.
    let out = session_until(
        &[r#"{"id":1,"method":"watch.start","params":{"interval_ms":100}}"#],
        &f.cfg,
        r#""initial":false"#,
    );
    assert_eq!(response(&out, 1)["result"]["watching"], true);
    let events: Vec<&Value> = out
        .iter()
        .filter(|m| m["event"] == "disks_changed")
        .collect();
    assert_eq!(
        events.len(),
        2,
        "un événement initial puis un changement : {events:?}"
    );
    assert_eq!(events[0]["initial"], true);
    assert_eq!(events[0]["report"]["disks"].as_array().unwrap().len(), 1);
    assert_eq!(events[1]["initial"], false);
    assert_eq!(events[1]["report"]["disks"].as_array().unwrap().len(), 2);
    // watch.stop arrête la surveillance.
    let f = fake(false, fixed());
    let out = session(
        &[
            r#"{"id":1,"method":"watch.start","params":{"interval_ms":100}}"#,
            r#"{"id":2,"method":"watch.stop"}"#,
        ],
        &f.cfg,
        Duration::from_millis(300),
    );
    assert_eq!(response(&out, 2)["result"]["was_watching"], true);
    // Arrêtée aussitôt, la surveillance a émis au plus l'événement initial, et plus rien ensuite.
    assert!(out.iter().filter(|m| m["event"] == "disks_changed").count() <= 1);
}

#[test]
fn echec_d_enumeration_rapporte_avec_son_code() {
    let broken: EnumerateFn = Arc::new(|| Err(EnumerateError::Unsupported { os: "test" }));
    let out = session(
        &[
            r#"{"id":1,"method":"disk.list"}"#,
            r#"{"id":2,"method":"diagnose"}"#,
        ],
        &fake(false, broken).cfg,
        Duration::ZERO,
    );
    assert_eq!(response(&out, 1)["error"]["code"], "E1002");
    // `diagnose` rapporte l'échec dans son rapport au lieu d'échouer.
    assert_eq!(response(&out, 2)["result"]["enumeration"]["ok"], false);
}

/// Une adresse passe par le même contrôle que dans la ligne de commande : son empreinte est exigée.
#[test]
fn disk_write_exige_l_empreinte_d_une_adresse() {
    let write = r#"{"id":1,"method":"disk.write","params":{"target":"usb-a","source":"https://a.org/x.iso","yes":true}}"#;
    let f = fake(false, fixed());
    let buffer = Buffer(Arc::new(Mutex::new(Vec::new())));
    let code = serve(
        Cursor::new(format!("{write}\n").into_bytes()),
        Box::new(buffer.clone()),
        &f.cfg,
    );
    assert_eq!(code, 0);
    let text = String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap();
    assert!(text.contains("--sha256"), "{text}");
}
