use bonk_hal::{Disk, Eligibility, FilterOptions, Lang, Warning, evaluate};

use crate::command::{ViewOptions, WriteView, equivalent_commands};

use super::*;

impl Model {
    pub fn requested(&self) -> Option<(&str, Requested<'_>)> {
        use crate::catalog::{CatalogState, ChoiceSource, find_id};
        let id = self.requested.as_deref()?;
        let state = match &self.catalog {
            CatalogState::NotLoaded | CatalogState::Loading => Requested::Waiting,
            CatalogState::Failed(_) => Requested::CatalogFailed,
            CatalogState::Ready(systems) => match find_id(systems, id) {
                Some((_, choice)) if matches!(choice.source, ChoiceSource::Download { .. }) => {
                    Requested::Ready(choice)
                }
                Some((_, choice)) => Requested::NotWritable(choice),
                None => Requested::Unknown,
            },
        };
        Some((id, state))
    }

    pub fn view_options(&self) -> ViewOptions {
        ViewOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
            show_all: self.show_all,
            offline: self.offline,
            incognito: self.incognito_at_start,
            notify: self.notify,
        }
    }

    /// Without the image, whose path may say who the user is.
    pub fn logged_equivalents(&self) -> String {
        equivalent_commands(self.view_options(), self.selected.as_deref(), None).join(" ; ")
    }

    pub fn equivalents(&self) -> Vec<String> {
        // The vault panel open on the selected disk: its command is the one to copy.
        if let Some(vault) = self.vault_equivalent() {
            return vec![vault];
        }
        let write = match &self.write {
            WriteState::Draft {
                image,
                hash,
                raw,
                mbr,
                ..
            } => Some(WriteView {
                image,
                hash,
                raw: *raw,
                mbr: *mbr,
            }),
            _ => None,
        };
        equivalent_commands(self.view_options(), self.selected.as_deref(), write)
    }

    pub fn entries(&self) -> Vec<(&Disk, Eligibility)> {
        let options = FilterOptions {
            show_internal: self.show_internal,
            allow_virtual: self.allow_virtual,
        };
        self.disks
            .iter()
            .map(|disk| (disk, evaluate(disk, options)))
            .filter(|(_, verdict)| self.show_all || verdict.is_writable())
            .collect()
    }

    pub fn offered(&self, id: &str) -> bool {
        self.entries()
            .iter()
            .any(|(disk, verdict)| disk.id == id && verdict.is_writable())
    }

    pub fn is_internal(&self, id: &str) -> bool {
        self.entries().iter().any(|(disk, verdict)| {
            disk.id == id
                && *verdict
                    == (Eligibility::Offered {
                        warning: Warning::InternalDisk,
                    })
        })
    }

    /// The panel may have changed since the prompt opened.
    pub fn internal_to_confirm(&self) -> Option<&Disk> {
        let id = self.write.disk_id().filter(|_| {
            self.confirming_internal && matches!(self.write, WriteState::Draft { .. })
        })?;
        self.disks
            .iter()
            .find(|disk| disk.id == id && self.is_internal(id))
    }

    pub fn status_line(&self, shown: usize) -> String {
        let every = REFRESH_EVERY.as_secs();
        let detected = self.disks.len();
        if self.error.is_some() {
            return match self.lang {
                Lang::En => format!("Detection failed. Automatic retry every {every} s."),
                Lang::Fr => {
                    format!(
                        "Détection en échec. Nouvelle tentative automatique toutes les {every} s."
                    )
                }
            };
        }
        if self.loading && !self.answered {
            return self
                .lang
                .pick("Detecting...", "Détection en cours...")
                .to_owned();
        }
        match self.lang {
            Lang::En => format!(
                "{shown} of {detected} detected disk(s) shown. Refreshes automatically every {every} s."
            ),
            Lang::Fr => format!(
                "{shown} disque(s) affiché(s) sur {detected} détecté(s). Actualisation automatique toutes les {every} s."
            ),
        }
    }
}
