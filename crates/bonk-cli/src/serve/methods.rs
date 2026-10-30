//! Méthodes immédiates : elles répondent sans lancer d'opération longue.

use bonk_hal::{Disk, FilterOptions};
use serde_json::{Value, json};

use crate::config::{self, ConfigAction, ConfigError};
use crate::errors::{self, Code};
use crate::{debug, diagnose, format, info, list, schema};

use super::params::{ConfigParams, DiagnoseParams, NameParams, TargetParams, ViewParams};
use super::protocol::{Failure, disk_not_found, failure, json_error, parse};
use super::{Config, Method, PROTOCOL};

pub(super) fn enumerate(cfg: &Config) -> Result<Vec<Disk>, Failure> {
    (cfg.enumerate)().map_err(|e| failure(Code::of_enumeration(&e), e))
}

pub(super) fn quick(method: Method, params: &Value, cfg: &Config) -> Result<Value, Failure> {
    match method {
        Method::Ping => Ok(json!({"pong": true})),
        Method::Version => Ok(json!({
            "version": env!("CARGO_PKG_VERSION"),
            "protocol": PROTOCOL,
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
        })),
        Method::Methods => Ok(Value::Array(
            Method::ALL
                .iter()
                .map(|m| json!({"method": m.name(), "about": m.about()}))
                .collect(),
        )),
        Method::DiskList => {
            let p: ViewParams = parse(params)?;
            let options = FilterOptions {
                show_internal: p.show_internal || cfg.view.show_internal,
                allow_virtual: p.allow_virtual || cfg.view.allow_virtual,
            };
            list::report_value(options, p.all || cfg.view.show_all, &enumerate(cfg)?)
                .map_err(json_error)
        }
        Method::DiskInfo => {
            let p: TargetParams = parse(params)?;
            let options = FilterOptions {
                show_internal: p.show_internal || cfg.view.show_internal,
                allow_virtual: p.allow_virtual || cfg.view.allow_virtual,
            };
            info::report_value(&p.target, options, &enumerate(cfg)?)
                .ok_or_else(|| disk_not_found(&p.target))?
                .map_err(json_error)
        }
        Method::DebugFilter => {
            let p: TargetParams = parse(params)?;
            let options = FilterOptions {
                show_internal: p.show_internal,
                allow_virtual: p.allow_virtual,
            };
            debug::filter_value(&p.target, options, &enumerate(cfg)?)
                .ok_or_else(|| disk_not_found(&p.target))?
                .map_err(json_error)
        }
        Method::Diagnose => {
            let p: DiagnoseParams = parse(params)?;
            diagnose::report_value(
                p.include_serials,
                &diagnose::Environment::system(),
                &(cfg.enumerate)(),
            )
            .map_err(json_error)
        }
        Method::Explain => {
            let p: NameParams = parse(params)?;
            errors::explain_value(p.code.as_deref()).ok_or_else(|| {
                failure(
                    Code::InvalidArgument,
                    errors::unknown_code_message(p.code.as_deref().unwrap_or_default()),
                )
            })
        }
        Method::Schema => {
            let p: NameParams = parse(params)?;
            match p.name {
                None => Ok(json!({"schemas": schema::names()})),
                Some(name) => schema::value(&name)
                    .ok_or_else(|| failure(Code::InvalidArgument, schema::unknown_message(&name))),
            }
        }
        Method::Capabilities => {
            format::report_value(bonk_hal::platform::current().capabilities()).map_err(json_error)
        }
        Method::ConfigList | Method::ConfigGet | Method::ConfigSet | Method::ConfigReset => {
            config_method(method, params, cfg)
        }
        Method::WatchStart
        | Method::WatchStop
        | Method::Cancel
        | Method::DiskWrite
        | Method::DiskVerify
        | Method::DiskRepairGpt
        | Method::DiskFormat => unreachable!("traité par `handle`"),
    }
}

pub(super) fn config_method(
    method: Method,
    params: &Value,
    cfg: &Config,
) -> Result<Value, Failure> {
    let p: ConfigParams = parse(params)?;
    let need = |name: &str, value: Option<String>| {
        value.ok_or_else(|| failure(Code::ProtocolError, format!("paramètre « {name} » requis")))
    };
    let action = match method {
        Method::ConfigList => ConfigAction::List { json: true },
        Method::ConfigGet => ConfigAction::Get {
            key: need("key", p.key)?,
        },
        Method::ConfigSet => ConfigAction::Set {
            key: need("key", p.key)?,
            value: need("value", p.value)?,
        },
        _ => ConfigAction::Reset { key: p.key },
    };
    let mut out = Vec::new();
    match config::run(&action, cfg.config_path.as_deref(), &cfg.vars, &mut out) {
        Ok(()) => {
            let text = String::from_utf8_lossy(&out).into_owned();
            if method == Method::ConfigList {
                serde_json::from_str(&text).map_err(json_error)
            } else {
                Ok(json!({"output": text.trim_end()}))
            }
        }
        Err(ConfigError::Invalid(m)) => Err(failure(Code::ConfigInvalid, m)),
        Err(ConfigError::Output(e)) => Err(failure(Code::OutputFailed, e)),
    }
}
