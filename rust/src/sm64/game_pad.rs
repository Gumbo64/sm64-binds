use serde::{Deserialize, Serialize};

/// Byte layout of the `rng_pad` WASM export, mirroring `OSContPad` in
/// `sm64-port/src/pc/pc_main.c`.
pub const GAME_PAD_SIZE: usize = 4;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamePad {
    pub button: u16,
    pub stick_x: i8,
    pub stick_y: i8,
}

impl GamePad {
    pub const fn new(button: u16, stick_x: i8, stick_y: i8) -> Self {
        GamePad {
            button,
            stick_x,
            stick_y,
        }
    }

    pub fn equals(&self, pad: &GamePad) -> bool {
        self == pad
    }

    /// Decodes the 4 byte little endian pad written by the WASM export.
    ///
    /// # Panics
    ///
    /// Panics if `data` is shorter than [`GAME_PAD_SIZE`].
    pub fn from_bytes(data: &[u8; GAME_PAD_SIZE]) -> Self {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buttons;

    #[test]
    fn from_bytes_decodes_little_endian_fields() {
        // A_BUTTON is 0x8000, which is [0x00, 0x80] little endian.
        let pad = GamePad::from_bytes(&[0x00, 0x80, 0x50, 0xB0]);
        assert_eq!(pad.button, buttons::A_BUTTON);
        assert_eq!(pad.stick_x, 80);
        assert_eq!(pad.stick_y, -80, "stick_y is signed");
    }

    #[test]
    fn from_bytes_round_trips() {
        let pad = GamePad::new(buttons::A_BUTTON | buttons::Z_TRIG, -80, 80);
        let bytes = [
            pad.button.to_le_bytes()[0],
            pad.button.to_le_bytes()[1],
            pad.stick_x as u8,
            pad.stick_y as u8,
        ];
        assert_eq!(GamePad::from_bytes(&bytes), pad);
    }

    #[test]
    fn default_is_a_neutral_pad() {
        assert_eq!(GamePad::default(), GamePad::new(0, 0, 0));
    }

    #[test]
    fn is_pressed_matches_every_single_bit() {
        // N64 face buttons live in the high byte, triggers and sticks in the low.
        for mask in [
            buttons::A_BUTTON,
            buttons::B_BUTTON,
            buttons::Z_TRIG,
            buttons::START_BUTTON,
            buttons::R_JPAD,
        ] {
            let mut pad = GamePad::default();
            pad.enable_button(mask);
            assert!(pad.is_pressed(mask), "0x{mask:04x} should be pressed");
            assert_eq!(
                pad.is_pressed(!mask),
                pad.button == 0,
                "complement of 0x{mask:04x} must not report a stray press"
            );
        }
    }

    #[test]
    fn is_pressed_is_false_for_unheld_bits() {
        let pad = GamePad::new(buttons::A_BUTTON, 0, 0);
        assert!(pad.is_pressed(buttons::A_BUTTON));
        assert!(!pad.is_pressed(buttons::B_BUTTON));
    }

    #[test]
    fn enable_and_disable_button_round_trip() {
        let mut pad = GamePad::default();

        pad.enable_button(buttons::A_BUTTON);
        pad.enable_button(buttons::B_BUTTON);
        assert_eq!(pad.button, buttons::A_BUTTON | buttons::B_BUTTON);

        pad.disable_button(buttons::A_BUTTON);
        assert_eq!(pad.button, buttons::B_BUTTON);

        pad.disable_button(buttons::B_BUTTON);
        assert_eq!(pad, GamePad::default());
    }

    #[test]
    fn disable_button_leaves_other_buttons_alone() {
        let mut pad = GamePad::new(buttons::A_BUTTON | buttons::B_BUTTON, 0, 0);
        pad.disable_button(buttons::B_BUTTON);
        assert_eq!(pad.button, buttons::A_BUTTON);
    }

    #[test]
    fn equals_compares_all_three_fields() {
        let pad = GamePad::new(buttons::A_BUTTON, 1, 2);
        assert!(pad.equals(&GamePad::new(buttons::A_BUTTON, 1, 2)));
        assert!(!pad.equals(&GamePad::new(buttons::B_BUTTON, 1, 2)));
        assert!(!pad.equals(&GamePad::new(buttons::A_BUTTON, 9, 2)));
        assert!(!pad.equals(&GamePad::new(buttons::A_BUTTON, 1, 9)));
    }
}
