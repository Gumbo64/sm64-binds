import { getSM64WASM } from "../fileUpload.js"
import SM64_LIB from "./pkg/sm64.us.js"

import { GamePad, GamepadButtons } from "./GamePad.js"
import { GameState } from "./GameState.js"

export { GamePad, GameState, GamepadButtons }

async function instantiateWasmSM64(info, func) {
    const wasmBuffer = await getSM64WASM();
    const instance = await WebAssembly.instantiate(wasmBuffer, info);
    func(instance["instance"], instance["module"]);
}

class Sm64VisualEngine {
    constructor(game, memory, rng_config) {
        this.game = game;
        this.memory = memory
        this.rng_config = rng_config;
    }

    static async create(canvas, seed = NaN, rng_config = null) {
        let memory = new WebAssembly.Memory({
            initial: 10000,
            maximum: 10000,
        });
        let game = await SM64_LIB({
            "instantiateWasm": instantiateWasmSM64,
            "canvas": canvas,
            "wasmMemory": memory
        });

        if (rng_config !== null) {
            game._set_rng_config(
                rng_config.window_length,
                rng_config.random_amount,
                rng_config.random_burst_length,
                rng_config.A_prob,
                rng_config.B_prob,
                rng_config.Z_prob
            );
        }
        if (!isNaN(seed)) {
            game._set_rng_seed(seed);
        }
        game._main_func();

        return new Sm64VisualEngine(game, memory, rng_config);
    }

    mem_view(pointer, length) {
        return new DataView(this.memory.buffer, pointer, length);
    }

    mem_slice(pointer, length) {
        return this.memory.buffer.slice(pointer, pointer + length);
    }

    step_game(pad) {
        return this.game._step_game(pad.button, pad.stick_x, pad.stick_y);
    }

    /** Reads the controller's current input, applying RNG if it is enabled. */
    rng_pad(pad) {
        let pointer = this.game._rng_pad(pad.button, pad.stick_x, pad.stick_y);
        return GamePad.from_pointer(pointer, this);
    }

    get_controller_pad() {
        let pointer = this.game._get_controller_pad();
        return GamePad.from_pointer(pointer, this);
    }

    get_game_state() {
        let pointer = this.game._get_game_state();
        return new GameState(pointer, this);
    }

    set_audio_enabled(truth) {
        return this.game._set_audio_enabled(truth);
    }
}

export { Sm64VisualEngine };
