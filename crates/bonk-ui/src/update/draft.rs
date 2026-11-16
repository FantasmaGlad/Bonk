use super::*;

impl Model {
    /// The fields of the write panel. [`Self::apply`] sends no other message here.
    pub(super) fn apply_draft(&mut self, message: Message) -> Effects {
        let focus = match &message {
            Message::ImageInput(_) => Focus::Image,
            Message::HashInput(_) => Focus::Hash,
            Message::ToggleRaw(_) => Focus::Raw,
            Message::ToggleMbr(_) => Focus::Mbr,
            Message::BrowseImage => Focus::Browse,
            Message::ToggleAdvanced => Focus::Advanced,
            Message::ImagePicked(path) => return self.on_image_picked(path.clone()),
            _ => return Vec::new(),
        };
        self.focus = Some(focus);
        match (message, &mut self.write) {
            (Message::ImageInput(text), WriteState::Draft { image, .. }) => {
                *image = text;
                self.requested = None;
            }
            (Message::HashInput(text), WriteState::Draft { hash, .. }) => *hash = text,
            (Message::ToggleRaw(on), WriteState::Draft { raw, .. }) => *raw = on,
            (Message::ToggleMbr(on), WriteState::Draft { mbr, .. }) => *mbr = on,
            (Message::BrowseImage, _) => return vec![Effect::PickImage],
            (Message::ToggleAdvanced, _) => self.advanced = !self.advanced,
            _ => {}
        }
        Vec::new()
    }

    pub(super) fn on_image_picked(&mut self, path: Option<std::path::PathBuf>) -> Effects {
        if let Some(path) = path
            && matches!(self.write, WriteState::Draft { .. })
        {
            self.fill_draft(&ImageSource::File(path));
            self.requested = None;
        }
        Vec::new()
    }
}
