use super::*;

#[test]
fn ecriture_avec_progression_puis_resultat() {
    let source = TempImage::new("ecriture", 9 << 20);
    let Fake { device, cfg } = fake(false, fixed());
    // A 9 MiB image on an 8 MiB disk is too large; take one that fits.
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
    // Message order: ready, start, progress (with the request id), result.
    assert_eq!(out[1]["event"], "job_started");
    assert_eq!(out[1]["request_id"], 7);
    let progress: Vec<&Value> = out.iter().filter(|m| m["event"] == "progress").collect();
    assert!(progress.len() >= 4 && progress.iter().all(|p| p["request_id"] == 7));
    assert_eq!(out.last().unwrap()["id"], 7);
    let written = device.memory.lock().unwrap().as_bytes()[..6 << 20].to_vec();
    assert_eq!(written, std::fs::read(&small.0).unwrap());
    // Too large: refused with its code, the disk untouched.
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
    let empreinte_remplacee = r#"{"id":6,"method":"disk.write","params":{"target":"usb-a","image":"debian","sha256_file":"https://example.org/SHA256SUMS","yes":true}}"#;
    let f = fake(false, fixed());
    // Operations follow one another: one at a time, each in its session.
    for (id, request, code) in [
        (1, sans_yes, "E2004"),
        (2, interdit, "E2001"),
        (4, par_chemin, "E1003"),
        (5, sans_source, "E6001"),
        (
            6,
            empreinte_remplacee.to_owned(),
            Code::InvalidArgument.id(),
        ),
    ] {
        let out = session(&[&request], &f.cfg, Duration::ZERO);
        assert_eq!(response(&out, id)["error"]["code"], code, "{id}");
    }
    let out = session(&[&plan], &f.cfg, Duration::ZERO);
    assert_eq!(response(&out, 3)["result"]["event"], "plan");
    // None of these requests wrote anything.
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
    // The second operation is refused while the first runs.
    assert_eq!(response(&out, 2)["error"]["code"], "E6001");
    assert!(
        response(&out, 2)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("already running")
    );
    assert_eq!(response(&out, 3)["result"]["cancelling"], true);
    // The first is cancelled cleanly: its own code, partial drive.
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
    // Without a running operation, cancel says there is nothing to cancel.
    let idle = quick_session(&[r#"{"id":1,"method":"cancel"}"#]);
    assert_eq!(response(&idle, 1)["result"]["cancelling"], false);
}

/// Input the client keeps open: nothing comes after the requests. It closes by itself after `deadline`, so a
/// service that never stops fails the test instead of hanging it.
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

/// Ctrl+C or SIGTERM: the running operation is cancelled between two blocks, its response goes out, and the
/// service returns the cancel code without waiting for the client to close the input.
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
    // The session ends without waiting for the ~7 slow blocks, and the operation is reported cancelled.
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(response(&out, 1)["error"]["code"], "E4003");
}

/// An address goes through the same check as on the command line: its hash is required.
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

/// The vault through the service, on an image file: prepared, listed, then GRUB found current.
#[test]
fn le_coffre_par_le_service() {
    use crate::job_tests::vault::{BASE, GRUB, served_catalog};

    let mut f = fake(false, fixed());
    f.cfg.catalog = served_catalog(Some(GRUB)).access();
    let image = std::env::temp_dir().join(format!("bonk-serve-coffre-{}.img", std::process::id()));
    let _ = std::fs::remove_file(&image);
    let cache =
        std::env::temp_dir().join(format!("bonk-serve-coffre-cache-{}", std::process::id()));
    let target = format!("{:?}", image.display().to_string());
    let cache_arg = format!("{:?}", cache.display().to_string());
    let request = |id: u64, method: &str, extra: &str| {
        format!(
            r#"{{"id":{id},"method":"{method}","params":{{"target":{target},"simulate":true,"catalog_url":"{BASE}","cache_dir":{cache_arg}{extra}}}}}"#
        )
    };
    let run = |id: u64, method: &str, extra: &str| {
        let out = session_until(
            &[&request(id, method, extra)],
            &f.cfg,
            &format!("\"id\":{id}"),
        );
        response(&out, id).clone()
    };
    let init = run(1, "vault.init", r#","size":268435456,"yes":true"#);
    assert_eq!(init["result"]["verified"], true, "{init}");
    assert_eq!(init["result"]["grub"]["version"], "2.14", "{init}");
    let list = run(2, "vault.list", "");
    assert_eq!(list["result"]["images"], serde_json::json!([]), "{list}");
    let update = run(3, "vault.update", "");
    assert_eq!(update["result"]["up_to_date"], true, "{update}");
    let missing = run(4, "vault.add", r#","yes":true"#);
    assert_eq!(missing["error"]["code"], "E6001", "{missing}");
    let _ = std::fs::remove_file(&image);
    let _ = std::fs::remove_dir_all(&cache);
}

/// An operation thread that panicked never answered: the service must not exit as if all went well.
#[test]
fn une_operation_qui_panique_donne_une_sortie_en_erreur() {
    let job = |panics: bool| JobHandle {
        cancel: Arc::new(AtomicBool::new(false)),
        thread: std::thread::spawn(move || assert!(!panics, "panique simulée")),
    };
    let mut state = State {
        job: Some(job(true)),
        ..State::default()
    };
    assert_eq!(shut_down(&mut state, false), EXIT_RUNTIME_ERROR);
    assert!(state.job.is_none());
    let mut calm = State {
        job: Some(job(false)),
        ..State::default()
    };
    assert_eq!(shut_down(&mut calm, true), EXIT_CANCELLED);
    assert_eq!(shut_down(&mut State::default(), false), 0);
}
