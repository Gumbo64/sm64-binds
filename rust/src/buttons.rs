//! N64 controller button masks, as reported by the WASM `rng_pad` export.

pub const A_BUTTON: u16 = 0x8000;
pub const B_BUTTON: u16 = 0x4000;
pub const L_TRIG: u16 = 0x0020;
pub const R_TRIG: u16 = 0x0010;
pub const Z_TRIG: u16 = 0x2000;
pub const START_BUTTON: u16 = 0x1000;
pub const U_JPAD: u16 = 0x0800;
pub const L_JPAD: u16 = 0x0200;
pub const R_JPAD: u16 = 0x0100;
pub const D_JPAD: u16 = 0x0400;
pub const U_CBUTTONS: u16 = 0x0008;
pub const L_CBUTTONS: u16 = 0x0002;
pub const R_CBUTTONS: u16 = 0x0001;
pub const D_CBUTTONS: u16 = 0x0004;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every mask must be a distinct single bit.
    #[test]
    fn masks_are_distinct_single_bits() {
        let all = [
            A_BUTTON,
            B_BUTTON,
            L_TRIG,
            R_TRIG,
            Z_TRIG,
            START_BUTTON,
            U_JPAD,
            L_JPAD,
            R_JPAD,
            D_JPAD,
            U_CBUTTONS,
            L_CBUTTONS,
            R_CBUTTONS,
            D_CBUTTONS,
        ];
        let mut seen = 0u32;
        for mask in all {
            assert!(mask.count_ones() == 1, "0x{mask:04x} is not a single bit");
            assert!(
                seen & mask as u32 == 0,
                "0x{mask:04x} collides with an earlier mask"
            );
            seen |= mask as u32;
        }
    }

    #[test]
    fn button_bits_sit_in_the_expected_byte() {
        // The low byte holds the triggers, the high byte the face buttons,
        // START and the D-pad. Both are part of the u16 button field.
        for mask in [A_BUTTON, B_BUTTON, Z_TRIG, START_BUTTON, U_JPAD, D_JPAD] {
            assert_eq!(
                mask & 0x00FF,
                0,
                "0x{mask:04x} should not set any low byte bit"
            );
            assert!(
                mask & 0xFF00 != 0,
                "0x{mask:04x} should set a high byte bit"
            );
        }
        for mask in [
            L_TRIG, R_TRIG, U_CBUTTONS, L_CBUTTONS, R_CBUTTONS, D_CBUTTONS,
        ] {
            assert_eq!(mask >> 8, 0x00, "0x{mask:04x} should be in the low byte");
        }
    }
}
