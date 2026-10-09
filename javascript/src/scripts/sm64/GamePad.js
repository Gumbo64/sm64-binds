// Controller input, mirroring the WASM `rng_pad` export layout:
//   u16 button; s8 stick_x; s8 stick_y; u8 errnum;
// See sm64-port/include/PR/os_cont.h.

/** Byte layout of the `rng_pad` export, mirroring `OSContPad`. */
export const GAME_PAD_SIZE = 4;

/** N64 controller button masks. */
export const GamepadButtons = Object.freeze({
    A_BUTTON: 0x8000,
    B_BUTTON: 0x4000,
    L_TRIG: 0x0020,
    R_TRIG: 0x0010,
    Z_TRIG: 0x2000,
    START_BUTTON: 0x1000,
    U_JPAD: 0x0800,
    L_JPAD: 0x0200,
    R_JPAD: 0x0100,
    D_JPAD: 0x0400,
    U_CBUTTONS: 0x0008,
    L_CBUTTONS: 0x0002,
    R_CBUTTONS: 0x0001,
    D_CBUTTONS: 0x0004,
});

export class GamePad {
    constructor(button = 0, stick_x = 0, stick_y = 0) {
        this.button = button;
        this.stick_x = stick_x;
        this.stick_y = stick_y;
    }

    equals(pad) {
        if (!(pad instanceof GamePad)) {
            return false;
        }
        return this.button === pad.button
            && this.stick_x === pad.stick_x
            && this.stick_y === pad.stick_y;
    }

    /** Decodes the 4 byte little endian pad written by the WASM export. */
    static from_pointer(pointer, engine) {
        const view = engine.mem_view(pointer, GAME_PAD_SIZE);
        return new GamePad(
            view.getUint16(0, true),
            view.getInt8(2),
            view.getInt8(3),
        );
    }

    is_pressed(button_mask) {
        return (this.button & button_mask) !== 0;
    }

    enable_button(button_mask) {
        this.button |= button_mask;
    }

    disable_button(button_mask) {
        this.button &= ~button_mask;
    }

    clone() {
        return new GamePad(this.button, this.stick_x, this.stick_y);
    }
}