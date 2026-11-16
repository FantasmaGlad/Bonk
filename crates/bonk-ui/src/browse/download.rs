use bonk_core::write::Phase;

use crate::catalog::ChoiceSource;
use crate::event::{Event, Outcome};
use crate::model::{Effect, Effects, Model};

/// One catalog image downloaded without being written; one at a time.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadState {
    Running {
        id: String,
        pid: Option<u32>,
        step: Option<Step>,
    },
    Finished {
        id: String,
        outcome: Outcome,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Step {
    pub phase: Phase,
    pub done: u64,
    pub total: Option<u64>,
    pub rate: f64,
}

impl DownloadState {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::Running { id, .. } | Self::Finished { id, .. } => id,
        }
    }

    #[must_use]
    pub fn running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }
}

#[must_use]
pub fn download_args(id: &str, proxy: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["catalog", "download", id, "--progress", "json"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    if let Some(proxy) = proxy {
        args.extend(["--proxy".to_owned(), proxy.to_owned()]);
    }
    args
}

impl Model {
    pub(super) fn start_download(&mut self, id: String) -> Effects {
        let busy = self
            .browse
            .download
            .as_ref()
            .is_some_and(DownloadState::running);
        let downloadable = self.opened_system().is_some_and(|system| {
            system.choices.iter().any(|choice| {
                choice.id == id && matches!(choice.source, ChoiceSource::Download { .. })
            })
        });
        if busy || !downloadable {
            return Vec::new();
        }
        let args = download_args(&id, self.proxy());
        log::info!(target: "bonk::gui", image = id.as_str(); "downloading without writing");
        self.browse.download = Some(DownloadState::Running {
            id,
            pid: None,
            step: None,
        });
        vec![Effect::RunDownload(args)]
    }

    pub(super) fn cancel_download(&self) -> Effects {
        match &self.browse.download {
            Some(DownloadState::Running { pid: Some(pid), .. }) => vec![Effect::CancelWrite(*pid)],
            _ => Vec::new(),
        }
    }

    pub(super) fn download_event(&mut self, event: Event) -> Effects {
        let Some(DownloadState::Running { id, pid, step }) = &mut self.browse.download else {
            return Vec::new();
        };
        match event {
            Event::Started(started) => *pid = Some(started),
            Event::Progress {
                phase,
                done,
                total,
                rate,
            } => {
                *step = Some(Step {
                    phase,
                    done,
                    total,
                    rate,
                });
            }
            Event::Finished(outcome) => {
                let kept = matches!(outcome, Outcome::Downloaded { .. });
                self.browse.download = Some(DownloadState::Finished {
                    id: id.clone(),
                    outcome,
                });
                return if kept {
                    vec![Effect::ListKept]
                } else {
                    Vec::new()
                };
            }
        }
        Vec::new()
    }
}
