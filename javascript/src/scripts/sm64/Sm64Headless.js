// The headless build of the game (what Rust, Python and the nodes run) in
// JavaScript: no canvas, no GL, no audio, so it runs in a Web Worker or in
// Node. Meant for fast-forwarding a replay: step to a frame, export a
// portable state (portable.js) and load it into the rendering engine
// (Sm64VisualEngine.load_portable_state), which has the same game from there.
//
// The WASM imports four WASI functions (stdio and exit) and nothing else;
// they are stubs, like the Rust crate's.

import { GamePad } from "./GamePad.js";
import { GameState } from "./GameState.js";
import { PortableTranslator, BUILD_HEADLESS, portableInfo, wasmCodeId } from "./portable.js";

// Offsets into `struct GameState` (GameState.js): the fields events are read from.
const GS_NUM_STARS = 0;
const GS_COURSE = 48;
const GS_AREA = 56;
// `struct MarioState`: the action.
const MARIO_ACTION = 12;

const WASI = () => ({
    wasi_snapshot_preview1: {
        proc_exit: () => {},
        fd_write: () => 0,
        fd_close: () => 0,
        fd_seek: () => 0,
    },
});

export class Sm64HeadlessEngine {
    constructor(module, translator, codeId) {
        this.module = module;
        this.translator = translator;
        this.codeId = codeId;
        this._boot();
    }

    /**
     * `wasm`: the headless build's bytes (the ROM-decrypted, unzipped WASM).
     * Compiling is the slow part; `restart()` makes a fresh game from the
     * compiled module.
     */
    static async create(wasm) {
        const bytes = wasm instanceof WebAssembly.Module ? null : wasm;
        const module = bytes ? await WebAssembly.compile(bytes) : wasm;
        const translator = await PortableTranslator.load();
        const codeId = bytes ? await wasmCodeId(bytes) : translator.buildId(BUILD_HEADLESS);
        if (codeId !== translator.buildId(BUILD_HEADLESS)) {
            throw new Error(`this headless build (${codeId}) is not the one the translator is for (${translator.buildId(BUILD_HEADLESS)})`);
        }
        return new Sm64HeadlessEngine(module, translator, codeId);
    }

    _boot() {
        const instance = new WebAssembly.Instance(this.module, WASI());
        this.x = instance.exports;
        this.memory = this.x.memory;
        this.x.main_func();
        this.frame = 0;
    }

    /** A fresh game, at power-on. */
    restart() {
        this._boot();
    }

    mem_view(pointer, length) {
        return new DataView(this.memory.buffer, pointer, length);
    }

    step_game(pad) {
        this.x.step_game(pad.button, pad.stick_x, pad.stick_y);
    }

    /** The same, from a flat array [button, stick_x, stick_y, ...] (frame `i`). */
    step_flat(flat, i) {
        this.x.step_game(flat[3 * i], flat[3 * i + 1], flat[3 * i + 2]);
    }

    get_controller_pad() {
        return GamePad.from_pointer(this.x.get_controller_pad(), this);
    }

    get_game_state() {
        return new GameState(this.x.get_game_state(), this);
    }

    /** [stars, course, area] without building a GameState (cheap enough for every frame). */
    progress() {
        const v = new DataView(this.memory.buffer, this.x.get_game_state(), 76);
        return [v.getInt32(GS_NUM_STARS, true), v.getInt32(GS_COURSE, true), v.getInt32(GS_AREA, true)];
    }

    /** Mario's `action` (for comparing games). */
    mario_action() {
        return new DataView(this.memory.buffer).getUint32(this.x.get_mario_state() + MARIO_ACTION, true);
    }

    /** The game's state as a portable state. Throws where there is none (the intro's Mario head). */
    export_portable_state() {
        return this.translator.exportState(this.memory.buffer, BUILD_HEADLESS);
    }

    /** Makes this game the one a portable state (from either build) was exported from. */
    load_portable_state(bytes) {
        const t = this.translator;
        const info = portableInfo(bytes);
        const r = t.loadState(bytes, this.memory.buffer, BUILD_HEADLESS, (n) => this.x.malloc(n));
        const mem = new Uint8Array(this.memory.buffer); // after malloc: it may have grown the memory
        for (const p of r.patches) mem.set(p.bytes, p.addr);
        for (const a of r.stale) this.x.free?.(a);
        // The render walk's cache holds nodes of the old state: invalidate it.
        const c = t.constants(BUILD_HEADLESS);
        const dv = new DataView(this.memory.buffer);
        dv.setUint32(c.geoCacheGeneration, (dv.getUint32(c.geoCacheGeneration, true) + 1) >>> 0, true);
        dv.setUint32(c.geoCacheCount, 0, true);
        return { source: info.source, allocated: r.allocated, untranslated: r.untranslated };
    }
}
