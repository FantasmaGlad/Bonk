use std::path::PathBuf;

use bonk_core::write::Phase;

use crate::event::{Event, Outcome, VaultDone};
use crate::model::{Effect, Effects, Model, WriteState};

/// No vault on the disk: `bonk vault list` answers this code.
const NOT_A_VAULT: &str = "E7003";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VaultAction {
    List,
    Prepare,
    Add,
    Update,
}

impl VaultAction {
    fn name(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Prepare => "init",
            Self::Add => "add",
            Self::Update => "update",
        }
    }
}

/// The vault panel of the selected disk. Every action runs `bonk vault`, as writing runs `bonk write`.
#[derive(Debug, Default)]
pub enum VaultState {
    #[default]
    Closed,
    Busy {
        disk_id: String,
        action: VaultAction,
        pid: Option<u32>,
        phase: Option<Phase>,
        done: u64,
        total: Option<u64>,
    },
    /// The disk holds no vault: preparing one erases it.
    NotVault {
        disk_id: String,
    },
    /// Second step before erasing: the question names the disk.
    Confirming {
        disk_id: String,
    },
    Ready {
        disk_id: String,
        images: Vec<(String, u64)>,
    },
    Finished {
        disk_id: String,
        outcome: Outcome,
    },
}

impl VaultState {
    #[must_use]
    pub fn disk_id(&self) -> Option<&str> {
        match self {
            Self::Closed => None,
            Self::Busy { disk_id, .. }
            | Self::NotVault { disk_id }
            | Self::Confirming { disk_id }
            | Self::Ready { disk_id, .. }
            | Self::Finished { disk_id, .. } => Some(disk_id),
        }
    }

    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy { .. })
    }
}

#[derive(Debug, Clone)]
pub enum VaultMessage {
    /// Opens the panel for the selected disk and reads it.
    Open,
    Close,
    Prepare,
    ConfirmPrepare,
    Browse,
    /// `None`: closed without a choice.
    Picked(Option<PathBuf>),
    Update,
    Cancel,
    Event(Event),
}

/// The command a vault action runs; `extra` holds the ISO of `add`.
#[must_use]
pub fn vault_args(
    action: VaultAction,
    disk_id: &str,
    extra: Option<&str>,
    show_internal: bool,
    allow_virtual: bool,
) -> Vec<String> {
    let mut args = vec![
        "vault".to_owned(),
        action.name().to_owned(),
        disk_id.to_owned(),
    ];
    args.extend(extra.map(str::to_owned));
    if action != VaultAction::List {
        args.push("--yes".to_owned());
    }
    args.extend(["--progress", "json"].map(str::to_owned));
    if show_internal {
        args.push("--show-internal".to_owned());
    }
    if allow_virtual {
        args.push("--allow-virtual".to_owned());
    }
    args
}

impl Model {
    fn run_vault(&mut self, disk_id: String, action: VaultAction, extra: Option<&str>) -> Effects {
        let args = vault_args(
            action,
            &disk_id,
            extra,
            self.show_internal,
            self.allow_virtual,
        );
        log::info!(target: "bonk::gui", action = action.name(); "vault command");
        self.vault = VaultState::Busy {
            disk_id,
            action,
            pid: None,
            phase: None,
            done: 0,
            total: None,
        };
        vec![Effect::RunVault(args)]
    }

    /// One drive process at a time: a running write keeps the vault closed, and the reverse.
    pub fn apply_vault(&mut self, message: VaultMessage) -> Effects {
        let writing = matches!(self.write, WriteState::Running { .. });
        match message {
            VaultMessage::Open => match self.selected.clone() {
                Some(disk_id) if !writing && !self.vault.is_busy() => {
                    self.run_vault(disk_id, VaultAction::List, None)
                }
                _ => Vec::new(),
            },
            VaultMessage::Close if !self.vault.is_busy() => {
                self.vault = VaultState::Closed;
                Vec::new()
            }
            VaultMessage::Prepare => {
                if let VaultState::NotVault { disk_id } | VaultState::Ready { disk_id, .. } =
                    &self.vault
                {
                    self.vault = VaultState::Confirming {
                        disk_id: disk_id.clone(),
                    };
                }
                Vec::new()
            }
            VaultMessage::ConfirmPrepare => match &self.vault {
                VaultState::Confirming { disk_id } if !writing => {
                    let disk_id = disk_id.clone();
                    self.run_vault(disk_id, VaultAction::Prepare, None)
                }
                _ => Vec::new(),
            },
            VaultMessage::Browse if matches!(self.vault, VaultState::Ready { .. }) => {
                vec![Effect::PickVaultIso]
            }
            VaultMessage::Picked(Some(path)) => match &self.vault {
                VaultState::Ready { disk_id, .. } if !writing => {
                    let disk_id = disk_id.clone();
                    let iso = path.display().to_string();
                    self.run_vault(disk_id, VaultAction::Add, Some(&iso))
                }
                _ => Vec::new(),
            },
            VaultMessage::Update => match &self.vault {
                VaultState::Ready { disk_id, .. } if !writing => {
                    let disk_id = disk_id.clone();
                    self.run_vault(disk_id, VaultAction::Update, None)
                }
                _ => Vec::new(),
            },
            VaultMessage::Cancel => match &self.vault {
                VaultState::Busy { pid: Some(pid), .. } => vec![Effect::CancelWrite(*pid)],
                _ => Vec::new(),
            },
            VaultMessage::Event(event) => self.on_vault_event(event),
            VaultMessage::Close | VaultMessage::Browse | VaultMessage::Picked(None) => Vec::new(),
        }
    }

    fn on_vault_event(&mut self, event: Event) -> Effects {
        let VaultState::Busy {
            disk_id,
            action,
            pid,
            phase,
            done,
            total,
        } = &mut self.vault
        else {
            return Vec::new();
        };
        match event {
            Event::Started(new) => *pid = Some(new),
            Event::Progress {
                phase: p,
                done: d,
                total: t,
                ..
            } => (*phase, *done, *total) = (Some(p), d, t),
            Event::Finished(outcome) => {
                let disk_id = disk_id.clone();
                log::info!(target: "bonk::gui", outcome = format!("{outcome:?}").as_str(); "vault command finished");
                self.vault = match (*action, outcome) {
                    (VaultAction::List, Outcome::Vault(VaultDone::Listed(images))) => {
                        VaultState::Ready { disk_id, images }
                    }
                    (VaultAction::List, Outcome::Failed { code, .. })
                        if code.as_deref() == Some(NOT_A_VAULT) =>
                    {
                        VaultState::NotVault { disk_id }
                    }
                    (_, outcome) => VaultState::Finished { disk_id, outcome },
                };
            }
        }
        Vec::new()
    }

    /// The command the vault panel would run next, for the equivalent shown on screen.
    #[must_use]
    pub fn vault_equivalent(&self) -> Option<String> {
        let (action, disk_id, extra) = match &self.vault {
            VaultState::Confirming { disk_id } => (VaultAction::Prepare, disk_id, None),
            VaultState::Ready { disk_id, .. } => (VaultAction::Add, disk_id, Some("<image.iso>")),
            VaultState::Busy {
                disk_id, action, ..
            } => (*action, disk_id, None),
            _ => return None,
        };
        let args = vault_args(
            action,
            disk_id,
            extra,
            self.show_internal,
            self.allow_virtual,
        );
        let shown: Vec<&str> = args
            .iter()
            .map(String::as_str)
            .filter(|arg| !["--progress", "json"].contains(arg))
            .collect();
        Some(format!("bonk {}", shown.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::event::parse_line;

    fn model_on(disk: &str) -> Model {
        Model {
            selected: Some(disk.to_owned()),
            ..Model::default()
        }
    }

    fn finish(model: &mut Model, line: &str) {
        let event = parse_line(line).expect("ligne comprise");
        let _ = model.apply_vault(VaultMessage::Event(event));
    }

    #[test]
    fn un_disque_sans_coffre_se_prepare_en_deux_temps() {
        let mut model = model_on("usb-a");
        let effects = model.apply_vault(VaultMessage::Open);
        assert_eq!(
            effects,
            [Effect::RunVault(
                ["vault", "list", "usb-a", "--progress", "json"]
                    .map(str::to_owned)
                    .to_vec()
            )]
        );
        finish(
            &mut model,
            r#"{"event":"error","code":"E7003","message":"no Bonk vault on this disk"}"#,
        );
        assert!(matches!(model.vault, VaultState::NotVault { .. }));
        assert!(model.apply_vault(VaultMessage::ConfirmPrepare).is_empty());
        let _ = model.apply_vault(VaultMessage::Prepare);
        assert_eq!(
            model.vault_equivalent().as_deref(),
            Some("bonk vault init usb-a --yes")
        );
        let effects = model.apply_vault(VaultMessage::ConfirmPrepare);
        assert!(matches!(&effects[..], [Effect::RunVault(args)] if args[1] == "init"));
        finish(
            &mut model,
            r#"{"event":"result","vault_label":"BonkVault","verified":true}"#,
        );
        assert!(matches!(
            model.vault,
            VaultState::Finished {
                outcome: Outcome::Vault(VaultDone::Prepared),
                ..
            }
        ));
    }

    #[test]
    fn un_coffre_liste_ses_iso_puis_en_ajoute_une() {
        let mut model = model_on("usb-a");
        let _ = model.apply_vault(VaultMessage::Open);
        finish(
            &mut model,
            r#"{"event":"result","images":[{"path":"debian-live.iso","size_bytes":1500000000}]}"#,
        );
        let VaultState::Ready { images, .. } = &model.vault else {
            panic!("coffre listé attendu : {:?}", model.vault);
        };
        assert_eq!(images, &[("debian-live.iso".to_owned(), 1_500_000_000)]);
        assert_eq!(
            model.apply_vault(VaultMessage::Browse),
            [Effect::PickVaultIso]
        );
        let effects =
            model.apply_vault(VaultMessage::Picked(Some(PathBuf::from("/isos/arch.iso"))));
        assert!(
            matches!(&effects[..], [Effect::RunVault(args)] if args[1] == "add" && args[3] == "/isos/arch.iso")
        );
        let _ = model.apply_vault(VaultMessage::Event(Event::Started(42)));
        assert_eq!(
            model.apply_vault(VaultMessage::Cancel),
            [Effect::CancelWrite(42)]
        );
        finish(
            &mut model,
            r#"{"event":"result","name":"arch.iso","size_bytes":1300000000,"family":"arch","verified":true}"#,
        );
        assert!(matches!(
            &model.vault,
            VaultState::Finished { outcome: Outcome::Vault(VaultDone::Added { name, .. }), .. } if name == "arch.iso"
        ));
    }

    #[test]
    fn une_ecriture_en_cours_garde_le_coffre_ferme() {
        let mut model = model_on("usb-a");
        model.write = WriteState::Running {
            disk_id: "usb-a".to_owned(),
            pid: None,
            phase: Phase::Write,
            done: 0,
            total: None,
            rate: 0.0,
        };
        assert!(model.apply_vault(VaultMessage::Open).is_empty());
        assert!(matches!(model.vault, VaultState::Closed));
    }

    #[test]
    fn mise_a_jour_lue_dans_son_resultat() {
        assert_eq!(
            parse_line(r#"{"event":"result","up_to_date":false,"updated":true,"verified":true}"#),
            Some(Event::Finished(Outcome::Vault(VaultDone::Updated {
                changed: true
            })))
        );
        // A write result keeps its own reading.
        assert!(matches!(
            parse_line(r#"{"event":"result","write":{"bytes":5},"verify":{}}"#),
            Some(Event::Finished(Outcome::Done { bytes: 5, .. }))
        ));
    }
}
