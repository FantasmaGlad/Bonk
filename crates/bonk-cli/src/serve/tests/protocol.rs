use super::*;

/// Each name maps to one method, and `methods` returns them all.
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
fn erreurs_de_protocole_avec_leur_code() {
    let out = quick_session(&[
        "ceci n'est pas du json",
        r#"{"id":1,"method":"unknown"}"#,
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
            r#"{"id":5,"method":"config.set","params":{"key":"view.show_all","value":"yes"}}"#,
            r#"{"id":6,"method":"config.get","params":{"key":"view.show_all"}}"#,
            r#"{"id":7,"method":"config.list"}"#,
            r#"{"id":8,"method":"config.set","params":{"key":"lang","value":"de"}}"#,
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
    let settings = response(&out, 7)["result"]["settings"].clone();
    let show_all = settings
        .as_array()
        .into_iter()
        .flatten()
        .find(|setting| setting["key"] == "view.show_all");
    assert_eq!(
        show_all.map(|setting| &setting["value"]),
        Some(&serde_json::json!("true"))
    );
    assert_eq!(response(&out, 8)["error"]["code"], "E5002");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn une_requete_trop_longue_est_sautee_sans_etre_gardee() {
    let mut input =
        Cursor::new(b"{\"a\":1}\nxxxxxxxxxxxxxxxxxxxx\n{\"b\":2}\nyyyyyyyyyyyy".to_vec());
    let lines: Vec<_> = std::iter::from_fn(|| next_line(&mut input, 10)).collect();
    assert!(matches!(&lines[..], [
        Incoming::Line(a),
        Incoming::TooLong,
        Incoming::Line(b),
        Incoming::TooLong,
    ] if a == "{\"a\":1}\n" && b == "{\"b\":2}\n"));
}
