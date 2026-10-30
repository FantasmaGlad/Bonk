//! Enveloppes du protocole : requête, réponse, erreur, et écriture d'un message par ligne.

use std::io::Write;
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::Code;

/// Une requête du protocole.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct Request {
    /// Identifiant libre, rendu tel quel dans la réponse.
    #[serde(default)]
    pub(super) id: Value,
    pub(super) method: String,
    #[serde(default)]
    pub(super) params: Value,
}

/// Erreur d'une réponse.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct ErrorBody {
    /// Code stable (`bonk explain <code>`).
    pub(super) code: &'static str,
    pub(super) exit_code: u8,
    pub(super) message: String,
}

/// Une réponse : `result` ou `error`.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Response {
    pub(super) id: Value,
    pub(super) result: Option<Value>,
    pub(super) error: Option<ErrorBody>,
}

pub(crate) fn schema_request() -> Value {
    serde_json::to_value(schemars::schema_for!(Request)).unwrap_or_default()
}

pub(crate) fn schema_response() -> Value {
    serde_json::to_value(schemars::schema_for!(Response)).unwrap_or_default()
}

pub(super) struct Failure {
    pub(super) code: Code,
    pub(super) message: String,
}

pub(super) fn failure(code: Code, message: impl std::fmt::Display) -> Failure {
    Failure {
        code,
        message: message.to_string(),
    }
}

/// Flux de sortie partagé : chaque message est une ligne écrite d'un seul tenant.
#[derive(Clone)]
pub(super) struct Out(pub(super) Arc<Mutex<Box<dyn Write + Send>>>);

pub(super) fn parse<T: DeserializeOwned>(params: &Value) -> Result<T, Failure> {
    let value = if params.is_null() {
        json!({})
    } else {
        params.clone()
    };
    serde_json::from_value(value)
        .map_err(|e| failure(Code::ProtocolError, format!("paramètres invalides : {e}")))
}

pub(super) fn json_error(e: serde_json::Error) -> Failure {
    failure(Code::OutputFailed, e)
}

pub(super) fn disk_not_found(target: &str) -> Failure {
    failure(
        Code::DiskNotFound,
        format!("disque introuvable : « {target} »"),
    )
}

impl Out {
    pub(super) fn send(&self, value: &Value) {
        if let Ok(mut writer) = self.0.lock() {
            let _ = writeln!(writer, "{value}");
            let _ = writer.flush();
        }
    }

    pub(super) fn respond(&self, id: &Value, outcome: Result<Value, Failure>) {
        let message = match outcome {
            Ok(result) => json!({"id": id, "result": result}),
            Err(f) => {
                json!({"id": id, "error": {"code": f.code.id(), "exit_code": f.code.exit(), "message": f.message}})
            }
        };
        self.send(&message);
    }
}
