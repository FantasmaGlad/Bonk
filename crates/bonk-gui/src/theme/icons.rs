use std::cell::RefCell;
use std::collections::HashMap;

use iced::Color;
use iced::widget::image::Handle;

const SIDE: u32 = 64;
// Alpha only: tinted to the palette, so the glyph keeps its contrast in every theme.
const INCOGNITO: &[u8; (SIDE * SIDE) as usize] = include_bytes!("../../assets/incognito-64.a8");

thread_local! {
    // One texture per colour: a new handle each frame would upload it again.
    static TINTED: RefCell<HashMap<[u8; 4], Handle>> = RefCell::default();
}

pub(crate) fn incognito(color: Color) -> Handle {
    let key = color.into_rgba8();
    TINTED.with(|tinted| {
        tinted
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| tint(key))
            .clone()
    })
}

fn tint([r, g, b, _]: [u8; 4]) -> Handle {
    let pixels = INCOGNITO
        .iter()
        .flat_map(|&a| [r, g, b, a])
        .collect::<Vec<u8>>();
    Handle::from_rgba(SIDE, SIDE, pixels)
}
