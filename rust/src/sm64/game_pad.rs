use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamePad {
    pub button: u16,
    pub stick_x: i8,
    pub stick_y: i8,
}

impl GamePad {
    pub const fn new(button: u16, stick_x: i8, stick_y: i8) -> Self {
        GamePad { button, stick_x, stick_y }
    }

    pub fn equals(&self, pad: &GamePad) -> bool {
        self == pad
    }

    pub fn from_bytes(data: &[u8;4]) -> Self {
        let button = u16::from_le_bytes(data[0..2].try_into().unwrap());
        let stick_x = data[2] as i8;
        let stick_y = data[3] as i8;
        GamePad::new(button, stick_x, stick_y)
    }

    pub const fn is_pressed(&self, button_mask: u16) -> bool {
        (self.button & button_mask) != 0
    }

    pub fn enable_button(&mut self, button_mask: u16) {
        self.button |= button_mask;
    }

    pub fn disable_button(&mut self, button_mask: u16) {
        self.button &= !button_mask;
    }
}

impl Default for GamePad {
    fn default() -> Self {
        GamePad::new(0, 0, 0)
    }
}
