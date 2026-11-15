use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use bonk_hal::Lang;
use bonk_ui::command::ViewOptions;
use bonk_ui::event::Event;
use bonk_ui::model::{Effect, Model};
use serde_json::{Value, json};

mod drives;
mod effects;
mod messages;
mod state;
#[cfg(test)]
mod tests;

pub(crate) use drives::{partitions, partitions_of};
pub use state::state;

use effects::{draft_of, effects_out};
use messages::{drive_event, message_of};

pub(crate) static CANCEL: AtomicBool = AtomicBool::new(false);
pub(crate) static EVENTS: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());

pub(crate) static MODEL: LazyLock<Mutex<Model>> =
    LazyLock::new(|| Mutex::new(Model::new(ViewOptions::default(), Lang::En).0));

/// A panic while holding a lock poisons it: refusing access afterwards would leave the screen dead until the
/// app restarts. The state is taken as it was; a running write has its own checks (read-back, hash), and the
/// worst case is a screen to refresh.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A link another app opened Bonk with. Only a catalog image name gets through: it chooses the image, and
/// the user still picks the drive and starts the write.
fn open_link(model: &mut Model, input: &Value) -> Vec<Effect> {
    if input["type"] != "open" {
        return Vec::new();
    }
    match input["link"].as_str().map(bonk_catalog::link::image_id) {
        Some(Ok(id)) => model.request_image(id.to_owned()),
        _ => Vec::new(),
    }
}

/// A message, or just a look at the state with `{}`: the state and the effects to perform.
#[must_use]
pub fn dispatch(input: &str) -> String {
    let mut model = lock(&MODEL);
    let input: Value = serde_json::from_str(input).unwrap_or_default();
    let mut effects = Vec::new();
    let pending: Vec<Event> = lock(&EVENTS).drain(..).collect();
    for event in pending {
        let message = drive_event(&model, event);
        effects.extend(model.update(message));
    }
    let draft = draft_of(&model);
    if let Some(message) = message_of(&input, &model) {
        effects.extend(model.update(message));
    }
    effects.extend(open_link(&mut model, &input));
    let effects = effects_out(&mut model, effects, draft);
    json!({"state": state(&model), "effects": effects}).to_string()
}
