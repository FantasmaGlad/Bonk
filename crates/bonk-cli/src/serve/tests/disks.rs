use super::*;

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
fn surveillance_des_disques() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&calls);
    let enumerate: EnumerateFn = Arc::new(move || {
        // The third enumeration sees a second key.
        if counter.fetch_add(1, Ordering::SeqCst) >= 2 {
            Ok(vec![usb("usb-a"), usb("usb-b")])
        } else {
            Ok(vec![usb("usb-a")])
        }
    });
    let f = fake(false, enumerate);
    // Wait for the second event instead of guessing a delay: on a loaded runner (macOS in CI) the 100 ms
    // interval stretches, and a fixed 700 ms wait only saw the initial event.
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
    // Stopped at once, the watch emitted at most the initial event, then nothing.
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
    // `diagnose` reports the failure in its report instead of failing.
    assert_eq!(response(&out, 2)["result"]["enumeration"]["ok"], false);
}
