//! Le service headless `bonk serve`, par le vrai binaire et de vrais tubes : un client écrit des
//! requêtes JSON sur l'entrée standard et lit les réponses et les événements sur la sortie.

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use serde_json::{Value, json};

mod common;

type Resultat = Result<(), Box<dyn std::error::Error>>;

struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: std::io::Lines<BufReader<std::process::ChildStdout>>,
}

impl Client {
    fn start() -> Result<Self, std::io::Error> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_bonk"))
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let lines =
            BufReader::new(child.stdout.take().ok_or(std::io::ErrorKind::BrokenPipe)?).lines();
        Ok(Self {
            child,
            stdin,
            lines,
        })
    }

    fn next(&mut self) -> Result<Value, Box<dyn std::error::Error>> {
        let line = self.lines.next().ok_or("le service s'est arrêté")??;
        Ok(serde_json::from_str(&line)?)
    }

    fn send(&mut self, request: &Value) -> Resultat {
        let stdin = self.stdin.as_mut().ok_or("entrée fermée")?;
        writeln!(stdin, "{request}")?;
        stdin.flush()?;
        Ok(())
    }

    /// Envoie une requête et rend tous les messages jusqu'à sa réponse (événements compris).
    fn call(
        &mut self,
        id: u64,
        method: &str,
        params: &Value,
    ) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        self.send(&json!({"id": id, "method": method, "params": params}))?;
        let mut messages = Vec::new();
        loop {
            let message = self.next()?;
            let done = message["id"] == id;
            messages.push(message);
            if done {
                return Ok(messages);
            }
        }
    }

    fn close(mut self) -> Result<Option<i32>, std::io::Error> {
        self.stdin = None;
        Ok(self.child.wait()?.code())
    }
}

#[test]
fn le_service_repond_et_s_arrete_a_la_fermeture_de_l_entree() -> Resultat {
    let mut client = Client::start()?;
    let ready = client.next()?;
    assert_eq!(ready["event"], "ready");
    assert_eq!(ready["protocol"], 1);
    let pong = client.call(1, "ping", &json!({}))?;
    assert_eq!(pong.last().ok_or("réponse")?["result"]["pong"], true);
    let methods = client.call(2, "methods", &json!({}))?;
    assert!(
        methods.last().ok_or("réponse")?["result"]
            .as_array()
            .is_some_and(|m| m.len() >= 19)
    );
    // Les erreurs portent leur code, le service continue.
    let inconnu = client.call(3, "inexistante", &json!({}))?;
    assert_eq!(inconnu.last().ok_or("réponse")?["error"]["code"], "E6001");
    let toujours = client.call(4, "ping", &json!({}))?;
    assert_eq!(toujours.last().ok_or("réponse")?["result"]["pong"], true);
    assert_eq!(client.close()?, Some(0));
    Ok(())
}

#[test]
fn ecriture_simulee_par_le_service() -> Resultat {
    let dir = std::env::temp_dir().join(format!("bonk-serve-e2e-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let (source, cible) = (dir.join("src.img"), dir.join("out.img"));
    let data: Vec<u8> = (0..(5 << 20)).map(|i| (i * 3 % 251) as u8).collect();
    fs::write(&source, &data)?;

    let mut client = Client::start()?;
    client.next()?; // ready
    let params = json!({
        "target": cible.display().to_string(), "simulate": true, "size": 6 << 20,
        "source": source.display().to_string(), "yes": true,
    });
    let messages = client.call(10, "disk.write", &params)?;
    let (reponse, evenements) = messages.split_last().ok_or("messages")?;
    assert_eq!(reponse["result"]["event"], "result");
    assert_eq!(reponse["result"]["write"]["bytes"], data.len());
    assert_eq!(evenements[0]["event"], "job_started");
    assert!(
        evenements
            .iter()
            .filter(|e| e["event"] == "progress")
            .count()
            >= 4
    );
    assert_eq!(&fs::read(&cible)?[..data.len()], data.as_slice());

    // Vérification, puis détection d'un octet modifié, par le même service.
    let ok = client.call(11, "disk.verify", &json!({"target": cible.display().to_string(), "simulate": true, "source": source.display().to_string()}))?;
    assert_eq!(ok.last().ok_or("réponse")?["result"]["event"], "result");
    let mut abime = fs::read(&cible)?;
    abime[777] ^= 1;
    fs::write(&cible, abime)?;
    let ko = client.call(12, "disk.verify", &json!({"target": cible.display().to_string(), "simulate": true, "source": source.display().to_string()}))?;
    let erreur = &ko.last().ok_or("réponse")?["error"];
    assert_eq!(
        (erreur["code"].as_str(), erreur["exit_code"].as_u64()),
        (Some("E4002"), Some(4)),
        "{erreur}"
    );

    // Un disque réel ne s'écrit pas sans confirmation, même par le service.
    let sans_yes = client.call(
        13,
        "disk.write",
        &json!({"target": "n-importe-quel-disque", "source": source.display().to_string()}),
    )?;
    assert!(sans_yes.last().ok_or("réponse")?["error"]["code"].is_string());
    assert_eq!(client.close()?, Some(0));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

/// Régression : le service répondait avant de libérer l'emplacement de l'opération, donc une requête
/// envoyée dès la réponse reçue pouvait être refusée (« déjà en cours »). Constaté sous Windows en CI.
#[test]
fn des_operations_enchainees_sans_attente_ne_sont_jamais_refusees() -> Resultat {
    let dir = std::env::temp_dir().join(format!("bonk-serve-chain-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let (source, cible) = (dir.join("src.img"), dir.join("out.img"));
    fs::write(&source, vec![7u8; 4096])?;
    let params = json!({
        "target": cible.display().to_string(), "simulate": true, "size": 8192,
        "source": source.display().to_string(), "yes": true,
    });
    let mut client = Client::start()?;
    client.next()?; // ready
    let ecrit = client.call(1, "disk.write", &params)?;
    assert_eq!(ecrit.last().ok_or("réponse")?["result"]["event"], "result");
    for id in 2..200 {
        let reponse = client.call(id, "disk.verify", &params)?;
        let derniere = reponse.last().ok_or("réponse")?;
        assert_eq!(
            derniere["result"]["event"], "result",
            "requête {id} : {derniere}"
        );
    }
    assert_eq!(client.close()?, Some(0));
    fs::remove_dir_all(&dir)?;
    Ok(())
}

#[test]
fn le_service_decrit_ses_messages_par_des_schemas() -> Resultat {
    for nom in ["serve-request", "serve-response"] {
        let sortie = Command::new(env!("CARGO_BIN_EXE_bonk"))
            .args(["schema", nom])
            .output()?;
        common::assert_code(&sortie, 0);
        let schema: Value = serde_json::from_slice(&sortie.stdout)?;
        assert_eq!(schema["type"], "object", "{nom}");
    }
    Ok(())
}
