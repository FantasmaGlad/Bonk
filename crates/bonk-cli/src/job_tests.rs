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

#[test]
fn ecriture_complete_dans_l_ordre_des_garde_fous() {
    let source = TempSource::new("ordre", 1 << 20);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    // Démontage d'abord, ouverture en écriture ensuite, jamais l'inverse.
    assert_eq!(
        scenario.log(),
        [
            "prepare:/dev/sdb",
            "unmount:usb-a",
            "open:/dev/sdb:ReadWrite",
            "drop_cache"
        ]
    );
    assert_eq!(scenario.written(1 << 20), source.bytes());
    assert!(out.contains("Démonté : /run/media/u/A"), "{out}");
    assert!(out.contains("Vérification : réussie"), "{out}");
}

#[test]
fn sans_yes_rien_n_est_touche() {
    let source = TempSource::new("sans-yes", 4096);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["write", "usb-a", &source.arg()]);
    assert_eq!(code, EXIT_USAGE);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2004]"),
        "{err}"
    );
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn dry_run_decrit_sans_rien_faire() {
    let source = TempSource::new("dry", 4096);
    let scenario = Scenario::new();
    let (code, out, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert_eq!(code, 0);
    assert!(
        out.contains("Plan (rien n'est écrit)") && out.contains("Démontage : /run/media/u/A"),
        "{out}"
    );
    assert!(
        out.contains("unmount, write, flush, verify, repair_gpt_if_needed"),
        "{out}"
    );
    assert!(scenario.log().is_empty());
    let (_, json, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run", "--json"]);
    let value: serde_json::Value = serde_json::from_str(&json).expect("JSON valide");
    assert_eq!(value["event"], "plan");
    assert_eq!(value["target"]["id"], "usb-a");
    assert_eq!(value["unmount"][0], "/run/media/u/A");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_disque_interdit_est_refuse_avant_toute_action() {
    let source = TempSource::new("interdit", 4096);
    let scenario = Scenario::new();
    // Même avec --yes et toutes les options ouvertes.
    let (code, out, err) = scenario.run(&[
        "write",
        "nvme-sys",
        &source.arg(),
        "--yes",
        "--show-internal",
        "--allow-virtual",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2001]"),
        "{err}"
    );
    assert!(err.contains("volume système"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn seul_l_identifiant_stable_designe_une_cible() {
    let source = TempSource::new("chemin", 4096);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "/dev/sdb", &source.arg(), "--yes"]);
    assert_eq!(code, 1);
    assert!(err.trim_end().ends_with("[E1003]"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_disque_remplace_entre_les_deux_enumerations_n_est_pas_ecrit() {
    let source = TempSource::new("remplace", 4096);
    let mut scenario = Scenario::new();
    // Première énumération : la clé choisie. Seconde (juste avant l'ouverture) : un autre numéro de série.
    scenario.enumerations = vec![vec![usb("S1")], vec![usb("S2")]];
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(
        out.is_empty() && err.trim_end().ends_with("[E2002]"),
        "{err}"
    );
    // Le démontage a eu lieu, l'ouverture jamais.
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
    assert!(scenario.written(4096).iter().all(|b| *b == 0));
}

#[test]
fn un_disque_devenu_interdit_entre_les_deux_enumerations_n_est_pas_ecrit() {
    let source = TempSource::new("devenu", 4096);
    let mut scenario = Scenario::new();
    let mut now_system = usb("S1");
    now_system.partitions = vec![partition("/dev/sdb1", &["/"], true)];
    scenario.enumerations = vec![vec![usb("S1")], vec![now_system]];
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED);
    assert!(err.trim_end().ends_with("[E2001]"), "{err}");
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
}

#[test]
fn echec_de_demontage_empeche_l_ouverture() {
    let source = TempSource::new("demontage", 4096);
    let mut scenario = Scenario::new();
    scenario.unmount = Err("démontage de /run/media/u/A impossible : target is busy".to_owned());
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, 1);
    assert!(
        err.contains("busy") && err.trim_end().ends_with("[E2003]"),
        "{err}"
    );
    assert_eq!(scenario.log(), ["prepare:/dev/sdb", "unmount:usb-a"]);
}

#[test]
fn droits_insuffisants_a_l_ouverture() {
    let source = TempSource::new("droits", 4096);
    let mut scenario = Scenario::new();
    scenario.open = Err(io::ErrorKind::PermissionDenied);
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, 1);
    assert!(err.trim_end().ends_with("[E4004]"), "{err}");
}

#[test]
fn image_connue_trop_grande_refusee_avant_le_demontage() {
    let source = TempSource::new("grande", DISK_SIZE + 512);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_SOURCE);
    assert!(err.trim_end().ends_with("[E3003]"), "{err}");
    assert!(
        scenario.log().is_empty(),
        "rien ne doit être démonté pour une image qui ne tient pas"
    );
}

#[test]
fn annulation_avant_toute_ecriture() {
    let source = TempSource::new("annule", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.cancel = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_CANCELLED);
    assert!(err.trim_end().ends_with("[E4003]"), "{err}");
    assert!(scenario.written(1 << 20).iter().all(|b| *b == 0));
}

#[test]
fn une_relecture_fausse_fait_echouer_la_verification() {
    let source = TempSource::new("defectueux", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.faulty_reads = true;
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
    // Sans vérification, la même écriture « réussit » : c'est bien la relecture qui détecte.
    let (code, _, _) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
        "--no-verify",
    ]);
    assert_eq!(code, 0);
}

#[test]
fn la_commande_verify_compare_sans_ecrire() {
    let source = TempSource::new("verify", 1 << 20);
    let scenario = Scenario::new();
    scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    scenario.log.borrow_mut().clear();
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert!(out.contains("Vérification réussie"), "{out}");
    // Relecture seule : pas de démontage, ouverture en lecture.
    assert_eq!(
        scenario.log(),
        ["prepare:/dev/sdb", "open:/dev/sdb:ReadOnly", "drop_cache"]
    );
    // Un octet qui change : la comparaison échoue avec le code dédié.
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(0, &[0xAA; 512])
        .expect("écriture de test");
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
}

#[test]
fn verify_json_et_progression_json() {
    let source = TempSource::new("json", 2 << 20);
    let scenario = Scenario::new();
    let (code, out, _) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "json",
    ]);
    assert_eq!(code, 0);
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).expect("chaque ligne est du JSON"))
        .collect();
    let (last, progress) = lines.split_last().expect("des lignes");
    assert_eq!(last["event"], "result");
    assert_eq!(last["write"]["bytes"], 2 << 20);
    assert!(
        progress.iter().all(|e| e["event"] == "progress"),
        "{progress:?}"
    );
    assert!(
        progress.iter().any(|e| e["phase"] == "write")
            && progress.iter().any(|e| e["phase"] == "verify")
    );
    let done: Vec<u64> = progress
        .iter()
        .filter(|e| e["phase"] == "write")
        .map(|e| e["bytes_done"].as_u64().unwrap_or(0))
        .collect();
    assert!(done.windows(2).all(|w| w[0] <= w[1]), "{done:?}");
}

/// Régression constatée sur une vraie clé : après `write`, la réparation du GPT modifie le MBR de
/// protection et l'en-tête principal ; un `verify` de la même image échouait donc (E4002) alors que le
/// contenu était intact.
#[test]
fn verify_d_une_image_gpt_reussit_apres_la_reparation_du_gpt() {
    use bonk_core::partition::testing::{LINUX, gpt_image};

    let image = gpt_image(2048, &[(64, 1023, LINUX, "donnees")]);
    let source = TempSource::from_bytes("gpt-verify", &image);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(out.contains("GPT : réparé"), "{out}");
    let (code, out, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    // Un octet modifié hors des champs que la réparation réécrit reste détecté.
    scenario
        .device
        .0
        .borrow_mut()
        .write_at(4096, &[0xAA; 512])
        .expect("écriture de test");
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!(code, EXIT_VERIFY_FAILED);
    assert!(err.trim_end().ends_with("[E4002]"), "{err}");
}

/// Régression constatée sur une vraie clé : la relecture de contrôle qui suit l'écriture lisait le cache
/// du noyau (2 193 Mo/s sur une clé USB 2.0), pas le support ; une écriture perdue par la clé n'aurait
/// pas été vue. Le cache doit être vidé avant toute relecture, à l'écriture comme à la vérification.
#[test]
fn la_relecture_de_controle_ne_lit_jamais_le_cache() {
    let source = TempSource::new("cache", 1 << 20);
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--progress",
        "none",
        "--no-repair-gpt",
    ]);
    assert_eq!((code, err.as_str()), (0, ""), "{out}");
    assert!(
        !scenario
            .log()
            .contains(&"lecture_avant_vidage_du_cache".to_owned()),
        "{:?}",
        scenario.log()
    );
    scenario.log.borrow_mut().clear();
    let (code, _, err) = scenario.run(&["verify", "usb-a", &source.arg(), "--progress", "none"]);
    assert_eq!((code, err.as_str()), (0, ""));
    assert_eq!(
        scenario.log(),
        ["prepare:/dev/sdb", "open:/dev/sdb:ReadOnly", "drop_cache"]
    );
}

#[test]
fn un_refus_d_autorisation_ne_demonte_rien() {
    let source = TempSource::new("refus", 1 << 20);
    let mut scenario = Scenario::new();
    scenario.prepare = Err("autorisation annulée".to_owned());
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("[E4004]"), "{err}");
    assert!(err.contains("autorisation annulée"), "{err}");
    let log = scenario.log.borrow();
    assert_eq!(*log, ["prepare:/dev/sdb"], "ni démontage ni ouverture");
}

/// Gzip fait à la main, blocs non compressés : le contenu est connu sans outil externe.
fn gzip_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
    let mut chunks = data.chunks(0xFFFF).peekable();
    while let Some(chunk) = chunks.next() {
        out.push(u8::from(chunks.peek().is_none()));
        let len = chunk.len() as u16;
        out.extend(len.to_le_bytes());
        out.extend((!len).to_le_bytes());
        out.extend(chunk);
    }
    out.extend(bonk_core::partition::crc32(data).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out
}

/// ISO hybride qui annonce `sectors` secteurs de 2 Kio, dans une image de 1 Mio.
fn cut_iso(sectors: u32) -> Vec<u8> {
    let mut image: Vec<u8> = (0..1usize << 20).map(|i| (i % 251) as u8).collect();
    image[446..510].fill(0);
    image[510] = 0x55;
    image[511] = 0xAA;
    let primary = 16 * 2048;
    image[primary] = 1;
    image[primary + 1..primary + 6].copy_from_slice(b"CD001");
    image[primary + 80..primary + 84].copy_from_slice(&sectors.to_le_bytes());
    image[primary + 128..primary + 130].copy_from_slice(&2048u16.to_le_bytes());
    image
}

#[test]
fn image_compressee_coupee_a_l_interieur_n_aboutit_pas_a_un_succes() {
    // Annonce 4 Mio pour 1 Mio réel : tient sur la cible, flux gzip complet.
    let source = TempSource::from_bytes("coupee-gz", &gzip_stored(&cut_iso(2048)));
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3007"), "{err}");
    // Le mode brut écrit ce qu'on lui donne, en le journalisant.
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes", "--mode", "raw"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn image_compressee_annoncee_plus_grande_que_la_cible_ne_touche_a_rien() {
    let source = TempSource::from_bytes("grande-gz", &gzip_stored(&cut_iso(100_000)));
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_SOURCE, "{err}");
    assert!(err.contains("E3003"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    let (code, _, _) = scenario.run(&["write", "usb-a", &source.arg(), "--dry-run"]);
    assert_eq!(code, EXIT_SOURCE);
}

#[test]
fn image_vide_refusee_meme_en_mode_brut() {
    for source in [
        TempSource::from_bytes("vide", &[]),
        TempSource::from_bytes("vide-gz", &gzip_stored(&[])),
    ] {
        let scenario = Scenario::new();
        let (code, _, err) =
            scenario.run(&["write", "usb-a", &source.arg(), "--yes", "--mode", "raw"]);
        assert_eq!(code, EXIT_SOURCE, "{err}");
        assert!(scenario.log().is_empty());
    }
}

#[test]
fn annuler_pendant_l_autorisation_laisse_la_cle_montee() {
    let source = TempSource::new("annule-autorisation", 4096);
    let mut scenario = Scenario::new();
    scenario.cancel_on_prepare = true;
    let (code, _, _) = scenario.run(&["write", "usb-a", &source.arg(), "--yes"]);
    assert_eq!(code, EXIT_CANCELLED);
    assert_eq!(scenario.log(), ["prepare:/dev/sdb"]);
}

#[test]
fn le_refus_d_elever_va_jusqu_au_demontage() {
    let source = TempSource::new("sans-elevation", 4096);
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "write",
        "usb-a",
        &source.arg(),
        "--yes",
        "--elevate",
        "never",
    ]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(scenario.seen_elevation.get(), Some(Elevation::Never));
}

#[test]
fn sans_reparation_demandee_le_gpt_n_est_pas_dit_sain() {
    let source = TempSource::new("sans-gpt", 4096);
    let (code, out, err) =
        Scenario::new().run(&["write", "usb-a", &source.arg(), "--yes", "--no-repair-gpt"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("GPT : contrôle non demandé"), "{out}");
    assert!(!out.contains("rien à réparer"), "{out}");
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

#[test]
fn disque_interne_refuse_avec_yes_seul() {
    let source = TempSource::new("interne-yes", 4096);
    let scenario = with_internal();
    let (code, _, err) =
        scenario.run(&["write", "sata-x", &source.arg(), "--yes", "--show-internal"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains(CONFIRMATION_REQUIRED), "{err}");
    assert!(err.contains("--force"), "{err}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
}

#[test]
fn disque_interne_ecrit_avec_force_sans_yes_ni_question() {
    let source = TempSource::new("interne-force", 4096);
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&[
        "write",
        "sata-x",
        &source.arg(),
        "--show-internal",
        "--force",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(scenario.log().iter().any(|l| l == "unmount:sata-x"));
}

#[test]
fn force_tient_lieu_de_yes_sur_une_cle() {
    let source = TempSource::new("force-cle", 4096);
    let (code, _, err) = Scenario::new().run(&["write", "usb-a", &source.arg(), "--force"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn force_n_ouvre_ni_un_disque_interne_cache_ni_un_disque_systeme() {
    let source = TempSource::new("force-interdit", 4096);
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&["write", "sata-x", &source.arg(), "--force"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(err.contains("E2001"), "{err}");
    let mut scenario = Scenario::new();
    scenario.enumerations = vec![vec![usb("S1"), system_disk()]];
    let (code, _, err) = scenario.run(&[
        "write",
        "nvme-sys",
        &source.arg(),
        "--force",
        "--show-internal",
    ]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn un_reglage_n_autorise_pas_a_ecrire_sur_un_disque_interne() {
    let source = TempSource::new("interne-reglage", 4096);
    let mut scenario = with_internal();
    scenario.vars = vec![("BONK_VIEW_SHOW_INTERNAL".to_owned(), "1".to_owned())];
    let (code, _, err) = scenario.run(&["write", "sata-x", &source.arg(), "--force"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(err.contains("E2001"), "{err}");
    assert!(scenario.log().is_empty());
    // Le réglage garde son rôle d'affichage.
    let (code, out, _) = scenario.run(&["list"]);
    assert_eq!(code, 0);
    assert!(out.contains("sata-x"), "{out}");
}

#[test]
fn reparer_un_disque_interne_exige_aussi_force() {
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&["repair-gpt", "sata-x", "--yes", "--show-internal"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains(CONFIRMATION_REQUIRED), "{err}");
}

/// Taille de la partition d'un GPT sur le disque simulé.
fn new_partition_bytes() -> u64 {
    let plan = bonk_core::format::table::plan(
        DISK_SIZE as u64,
        512,
        bonk_core::format::table::TableKind::Gpt,
    )
    .expect("plan");
    plan.sectors() * 512
}

/// Le disque après formatage, avec le système de fichiers que le système y voit.
fn usb_with_new_partition(fs: &str) -> Disk {
    let mut disk = usb("S1");
    let mut part = partition("/dev/sdb1", &[], false);
    part.size_bytes = new_partition_bytes();
    part.fs_type = Some(fs.to_owned());
    disk.partitions = vec![part];
    disk
}

#[test]
fn formater_une_cle_passe_par_tous_les_garde_fous_puis_relit() {
    let scenario = Scenario::new();
    let (code, out, err) = scenario.run(&["format", "usb-a", "--label", "cle", "--yes", "--json"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        scenario.log(),
        [
            "prepare:/dev/sdb",
            "unmount:usb-a",
            "open:/dev/sdb:ReadWrite",
            "drop_cache",
            "rescan:/dev/sdb"
        ]
    );
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["filesystem"], "fat32");
    assert_eq!(value["table"], "gpt");
    assert_eq!(value["label"], "CLE");
    assert_eq!(value["verified"], true);
    assert_eq!(value["written_by"], "bonk");
    let written = scenario.written(1024);
    assert_eq!(&written[512..520], b"EFI PART");
}

#[test]
fn formater_sans_confirmation_ou_en_plan_ne_touche_a_rien() {
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["format", "usb-a"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains("E2004"), "{err}");
    let (code, out, err) = scenario.run(&["format", "usb-a", "--dry-run"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("Plan"), "{out}");
    assert!(out.contains("fat32 (par défaut)"), "{out}");
    assert!(scenario.log().is_empty(), "{:?}", scenario.log());
    assert!(scenario.written(1024).iter().all(|b| *b == 0));
}

#[test]
fn formater_un_disque_systeme_ou_interne_obeit_au_filtre_et_a_force() {
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&["format", "nvme-sys", "--force", "--show-internal"]);
    assert_eq!(code, EXIT_TARGET_REFUSED, "{err}");
    assert!(scenario.log().is_empty());
    let scenario = with_internal();
    let (code, _, err) = scenario.run(&["format", "sata-x", "--yes", "--show-internal"]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(err.contains(CONFIRMATION_REQUIRED), "{err}");
    let (code, _, err) = scenario.run(&["format", "sata-x", "--force", "--show-internal"]);
    assert_eq!(code, 0, "{err}");
}

#[test]
fn etiquette_invalide_refusee_avant_tout_geste() {
    let scenario = Scenario::new();
    let (code, _, err) = scenario.run(&[
        "format",
        "usb-a",
        "--yes",
        "--label",
        "beaucoup trop longue",
    ]);
    assert_eq!(code, EXIT_USAGE, "{err}");
    assert!(scenario.log().is_empty());
}

fn exfat_capability(available: bool) -> Capability {
    if available {
        Capability::yes("format.exfat")
    } else {
        Capability::no(
            "format.exfat",
            Missing::MissingTool,
            "outil absent : mkfs.exfat",
        )
    }
}

#[test]
fn un_outil_absent_refuse_exfat_sans_toucher_au_disque() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(false)];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E4007"), "{err}");
    assert!(err.contains("mkfs.exfat"), "{err}");
    assert!(scenario.log().is_empty());
}

#[test]
fn exfat_ne_formate_jamais_l_ancienne_partition_encore_visible() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    // L'ancienne table reste visible un moment : même chemin /dev/sdb1, autre taille. C'est ce qu'a montré
    // l'essai sur la clé réelle.
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
        vec![usb_with_new_partition("exfat")],
    ];
    let (code, _, err) = scenario.run(&[
        "format", "usb-a", "--fs", "exfat", "--label", "Clé", "--yes",
    ]);
    assert_eq!(code, 0, "{err}");
    let log = scenario.log();
    assert!(log.contains(&"rescan:/dev/sdb".to_owned()), "{log:?}");
    assert_eq!(
        log.last().map(String::as_str),
        Some("format:/dev/sdb1:exfat:Clé")
    );
}

#[test]
fn exfat_sans_nouvelle_partition_ne_formate_rien() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("E4006"), "{err}");
    assert!(!scenario.log().iter().any(|l| l.starts_with("format:")));
}

#[test]
fn exfat_que_le_systeme_ne_reconnait_pas_est_un_echec() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![exfat_capability(true)];
    scenario.enumerations = vec![
        vec![usb("S1")],
        vec![usb("S1")],
        vec![usb_with_new_partition("")],
    ];
    let (code, _, err) = scenario.run(&["format", "usb-a", "--fs", "exfat", "--yes"]);
    assert_eq!(code, 1, "{err}");
    assert!(err.contains("ne reconnaît pas"), "{err}");
}

#[test]
fn capabilities_dit_le_pourquoi() {
    let mut scenario = Scenario::new();
    scenario.capabilities = vec![Capability::yes("write"), exfat_capability(false)];
    let (code, out, _) = scenario.run(&["capabilities"]);
    assert_eq!(code, 0);
    assert!(out.contains("write"), "{out}");
    assert!(
        out.contains("format.exfat   non : outil absent (outil absent : mkfs.exfat)"),
        "{out}"
    );
    let (code, out, _) = scenario.run(&["capabilities", "--json"]);
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["capabilities"][1]["missing"], "missing_tool");
}
