use super::*;

impl Model {
    /// A single offered disk and no choice yet: selecting it saves a click. Once per disk, so a deselection
    /// holds despite the refresh.
    pub(super) fn select_only_disk(&mut self) {
        if !self.auto_select || self.selected.is_some() {
            return;
        }
        let offered: Vec<String> = self
            .entries()
            .iter()
            .filter(|(_, verdict)| verdict.is_writable())
            .map(|(disk, _)| disk.id.clone())
            .collect();
        if let [only] = offered.as_slice()
            && self.auto_selected.as_ref() != Some(only)
        {
            self.auto_selected = Some(only.clone());
            self.selected = Some(only.clone());
        }
    }

    pub(super) fn on_disks_loaded(&mut self, disks: Vec<Disk>) -> Effects {
        log::debug!(target: "bonk::gui", disks = disks.len(); "enumeration result received");
        self.loading = false;
        self.answered = true;
        self.error = None;
        if self
            .selected
            .as_ref()
            .is_some_and(|id| !disks.iter().any(|d| &d.id == id))
        {
            self.selected = None;
        }
        self.disks = disks;
        self.select_only_disk();
        self.sync_write();
        self.fix_focus();
        self.load_menu()
    }

    pub(super) fn on_detection_failed(&mut self, erreur: String) -> Effects {
        log::warn!(target: "bonk::gui", reason = erreur.as_str(); "enumeration failed");
        self.loading = false;
        // Never a stale list shown as valid after a failure.
        self.disks.clear();
        self.selected = None;
        self.error = Some(erreur);
        self.sync_write();
        self.fix_focus();
        Vec::new()
    }
}
