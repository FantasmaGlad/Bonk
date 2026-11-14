use serde_json::Value;

use bonk_core::write::Phase;

use crate::command::{ViewOptions, WriteView};

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Process id, for cancelling.
    Started(u32),
    Progress {
        phase: Phase,
        done: u64,
        total: Option<u64>,
        rate: f64,
    },
    Finished(Outcome),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done {
        bytes: u64,
        verified: bool,
        gpt_repaired: bool,
        /// Windows drive: the loader certificate; the screen says what it means in its language.
        secure_boot: Option<bonk_core::pe::Signer>,
        /// Windows drive that also boots in legacy BIOS.
        bios_boot: bool,
    },
    Failed {
        /// Stable error code (`bonk explain`), when the process gave one.
        code: Option<String>,
        message: String,
    },
    Vault(VaultDone),
}

/// What a `bonk vault` command did, read from its JSON result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultDone {
    /// ISO images at the root: name and size.
    Listed(Vec<(String, u64)>),
    Prepared,
    Added {
        name: String,
        bytes: u64,
    },
    /// `false`: the boot partition already held the catalog's files.
    Updated {
        changed: bool,
    },
}

/// Each `vault` result has its own fields: the one present says which command answered.
fn vault_done(value: &Value) -> Option<VaultDone> {
    if let Some(images) = value["images"].as_array() {
        let images = images
            .iter()
            .filter_map(|image| {
                Some((
                    image["path"].as_str()?.to_owned(),
                    image["size_bytes"].as_u64()?,
                ))
            })
            .collect();
        return Some(VaultDone::Listed(images));
    }
    if value["vault_label"].is_string() {
        return Some(VaultDone::Prepared);
    }
    if let (Some(name), Some(bytes)) = (value["name"].as_str(), value["size_bytes"].as_u64()) {
        return Some(VaultDone::Added {
            name: name.to_owned(),
            bytes,
        });
    }
    let changed = value["updated"].as_bool()?;
    value["up_to_date"].as_bool()?;
    Some(VaultDone::Updated { changed })
}

pub fn write_args(
    disk_id: &str,
    write: WriteView<'_>,
    options: ViewOptions,
    internal_confirmed: bool,
    proxy: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = ["write", disk_id, write.image, "--yes", "--progress", "json"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    args.extend(write.flags());
    if let Some(proxy) = proxy {
        args.extend(["--proxy".to_owned(), proxy.to_owned()]);
    }
    if options.show_internal {
        args.push("--show-internal".to_owned());
    }
    if options.allow_virtual {
        args.push("--allow-virtual".to_owned());
    }
    // Confirmed in the window: `bonk write` then requires `--force`.
    if internal_confirmed {
        args.push("--force".to_owned());
    }
    args
}

pub fn parse_line(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    match value["event"].as_str()? {
        "progress" => Some(Event::Progress {
            phase: serde_json::from_value(value["phase"].clone()).ok()?,
            done: value["bytes_done"].as_u64()?,
            total: value["bytes_total"].as_u64(),
            rate: value["bytes_per_second"].as_f64().unwrap_or(0.0),
        }),
        "result" if value["write"].is_null() => {
            vault_done(&value).map(|done| Event::Finished(Outcome::Vault(done)))
        }
        "result" => Some(Event::Finished(Outcome::Done {
            bytes: value["write"]["bytes"].as_u64()?,
            verified: !value["verify"].is_null(),
            gpt_repaired: !value["gpt_repair"].is_null(),
            secure_boot: serde_json::from_value(value["extraction"]["signer"].clone()).ok(),
            bios_boot: value["extraction"]["firmware"]
                .as_array()
                .is_some_and(|firmware| firmware.iter().any(|f| f == "bios")),
        })),
        "error" => Some(Event::Finished(Outcome::Failed {
            code: value["code"].as_str().map(str::to_owned),
            message: value["message"].as_str()?.to_owned(),
        })),
        _ => None,
    }
}

/// A process that could not emit its `error` event (arguments refused, crash), read from stderr: the last
/// `bonk: error: message [Exxxx]` line.
pub fn parse_failure(stderr: &str, status: &str) -> Outcome {
    let line = stderr
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("bonk: error: "));
    match line {
        Some(text) => {
            let (message, code) = match text.rsplit_once(" [E") {
                Some((message, rest)) if rest.ends_with(']') && rest.len() == 5 => {
                    (message, Some(format!("E{}", &rest[..4])))
                }
                _ => (text, None),
            };
            Outcome::Failed {
                code,
                message: message.to_owned(),
            }
        }
        // Stopped without an error line (killed, crash): filed as a write failure, whose title the window
        // shows in its language; the detail follows the engine's message language.
        None => Outcome::Failed {
            code: Some("E4001".to_owned()),
            message: format!("the write process stopped ({status})"),
        },
    }
}
