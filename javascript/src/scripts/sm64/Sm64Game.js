import { getSM64WASM } from "../fileUpload.js"
import SM64_LIB from "./pkg/sm64.us.js"

import { GamePad, GamepadButtons } from "./GamePad.js"
import { GameState } from "./GameState.js"
import { PortableTranslator, BUILD_VISUAL, portableInfo, wasmCodeId } from "./portable.js"

export { wasmCodeId }

export { GamePad, GameState, GamepadButtons }

// What the engine being created needs from the WASM it was built from: its
// exports (malloc and free, which load a state into the game's heap) and its
// code's SHA-1 (which build it is, for save states).
let lastWasm = null;

async function instantiateWasmSM64(info, func) {
    const wasmBuffer = await getSM64WASM();
    const instance = await WebAssembly.instantiate(wasmBuffer, info);
    lastWasm = { exports: instance["instance"].exports, bytes: wasmBuffer };
    func(instance["instance"], instance["module"]);
}

const RAW_MAGIC = "SM64VISU";
const RAW_VERSION = 1;
const DELTA_VERSION = 2; // only the 4 KiB pages that differ from a base snapshot
const PAGE = 4096;
const RAW_HEADER = 44;

class Sm64VisualEngine {
    constructor(game, memory, wasm = null, translator = null, codeId = null) {
        this.game = game;
        this.memory = memory
        this.wasm = wasm;
        this.translator = translator;
        this.codeId = codeId;
    }

    static async create(canvas) {
        let memory = new WebAssembly.Memory({
            initial: 10000,
            maximum: 10000,
        });
        let game = await SM64_LIB({
            "instantiateWasm": instantiateWasmSM64,
            "canvas": canvas,
            "wasmMemory": memory
        });

        const wasm = lastWasm;
        lastWasm = null;
        game._main_func();

        let translator = null;
        let codeId = null;
        try {
            translator = await PortableTranslator.load();
            codeId = await wasmCodeId(wasm.bytes);
        } catch (e) {
            console.warn("sm64: portable states are unavailable:", e);
        }
        const engine = new Sm64VisualEngine(game, memory, wasm, translator, codeId);
        // The game as booted: a portable state is loaded into this (see load_portable_state).
        try {
            if (translator) engine._bootState = engine.save_state();
        } catch (e) {
            console.warn("sm64: no boot snapshot:", e);
        }
        return engine;
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

    // --- save states --------------------------------------------------------

    _translator() {
        if (this.translator === null) throw new Error("portable states are unavailable (the translator did not load)");
        if (this.codeId !== this.translator.buildId(BUILD_VISUAL)) {
            throw new Error(`this game build (${this.codeId}) is not the one the translator is for (${this.translator.buildId(BUILD_VISUAL)})`);
        }
        return this.translator;
    }

    /** Whether this engine can make and load portable states. */
    get portable_supported() {
        return this.translator !== null && this.codeId === this.translator.buildId(BUILD_VISUAL);
    }

    /**
     * A snapshot of the game, as bytes that only this build loads
     * ([`load_state`]): its stack and everything after the read-only data
     * (variables and heap), the way the headless build's save states work.
     * About 13 MB, and quick (a copy).
     *
     * The snapshot also holds the renderer's and SDL's memory, so it is only
     * meant for going back within the session that made it (seeking, undo),
     * not for storing: use [`export_portable_state`] for that. What the JS side
     * keeps (GL objects, the audio queue's timing) is not in it; loading
     * resets the texture cache, which is the one thing there that can
     * disagree with the restored game.
     */
    save_state() {
        const t = this._translator();
        const c = t.constants(BUILD_VISUAL);
        const end = t.memoryLength(this.memory.buffer, BUILD_VISUAL);
        const stackLen = 0x10000;
        const out = new Uint8Array(RAW_HEADER + stackLen + (end - c.dataStart));
        const dv = new DataView(out.buffer);
        out.set(new TextEncoder().encode(RAW_MAGIC), 0);
        dv.setUint32(8, RAW_VERSION, true);
        for (let i = 0; i < 20; i++) out[12 + i] = parseInt(this.codeId.substr(2 * i, 2), 16);
        dv.setUint32(32, stackLen, true);
        dv.setUint32(36, end - c.dataStart, true);
        const mem = new Uint8Array(this.memory.buffer);
        out.set(mem.subarray(0, stackLen), RAW_HEADER);
        out.set(mem.subarray(c.dataStart, end), RAW_HEADER + stackLen);
        return out;
    }

    /**
     * Like [`save_state`], but a delta against `base` (a full snapshot from
     * `save_state()`): only the 4 KiB pages that changed are kept, so a trail of
     * snapshots of a game that changes little between them costs little (a few
     * hundred KB each rather than 13 MB). Load it with `load_state(delta, base)`;
     * `base` has to be kept as long as its deltas are.
     */
    save_state_delta(base) {
        const full = this.save_state();
        const body = RAW_HEADER;
        if (full.length !== base.length) return full; // the heap grew: keep it whole
        const pages = [];
        for (let o = body; o < full.length; o += PAGE) {
            const e = Math.min(o + PAGE, full.length);
            let same = true;
            for (let k = o; k < e; k += 4) {
                if (full[k] !== base[k] || full[k + 1] !== base[k + 1] || full[k + 2] !== base[k + 2] || full[k + 3] !== base[k + 3]) {
                    same = false;
                    break;
                }
            }
            if (!same) pages.push(o);
        }
        const out = new Uint8Array(RAW_HEADER + 4 + pages.length * (4 + PAGE));
        out.set(full.subarray(0, RAW_HEADER), 0);
        const dv = new DataView(out.buffer);
        dv.setUint32(8, DELTA_VERSION, true);
        dv.setUint32(RAW_HEADER, pages.length, true);
        let w = RAW_HEADER + 4;
        for (const o of pages) {
            dv.setUint32(w, (o - body) / PAGE, true);
            out.set(full.subarray(o, Math.min(o + PAGE, full.length)), w + 4);
            w += 4 + PAGE;
        }
        return out;
    }

    /** Loads a snapshot made by [`save_state`] of this build (or by [`save_state_delta`], with its `base`). */
    load_state(bytes, base = null) {
        const t = this._translator();
        const c = t.constants(BUILD_VISUAL);
        const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        if (bytes.length < RAW_HEADER || String.fromCharCode(...bytes.subarray(0, 8)) !== RAW_MAGIC) {
            throw new Error("not a rendering build save state");
        }
        const version = dv.getUint32(8, true);
        if (version === DELTA_VERSION) {
            if (!base) throw new Error("a delta save state needs its base snapshot");
            const full = base.slice();
            const n = dv.getUint32(RAW_HEADER, true);
            let r = RAW_HEADER + 4;
            for (let k = 0; k < n; k++) {
                const o = RAW_HEADER + dv.getUint32(r, true) * PAGE;
                full.set(bytes.subarray(r + 4, r + 4 + Math.min(PAGE, full.length - o)), o);
                r += 4 + PAGE;
            }
            bytes = full;
        } else if (version !== RAW_VERSION) {
            throw new Error("unsupported save state version");
        }
        const dv2 = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
        const id = Array.from(bytes.subarray(12, 32), (b) => b.toString(16).padStart(2, "0")).join("");
        if (id !== this.codeId) throw new Error(`save state is from build ${id}, this game is build ${this.codeId}`);
        const stackLen = dv2.getUint32(32, true);
        const heapLen = dv2.getUint32(36, true);
        if (bytes.length !== RAW_HEADER + stackLen + heapLen) throw new Error("save state is truncated");
        const mem = new Uint8Array(this.memory.buffer);
        // The renderer's state describes the live GL context (the GL state it mirrors: depth
        // test, blending, viewport, bound shader; its shader programs; the window size), not the
        // game: overwriting it with a snapshot's makes it skip GL calls it believes done, and
        // the frames come out black or wrong (worst with the boot snapshot, taken before
        // anything was drawn). Keep it; only the texture cache is reset below.
        const keep = c.rendererSize ? mem.slice(c.renderer, c.renderer + c.rendererSize) : null;
        mem.set(bytes.subarray(RAW_HEADER, RAW_HEADER + stackLen), 0);
        mem.set(bytes.subarray(RAW_HEADER + stackLen), c.dataStart);
        if (keep) mem.set(keep, c.renderer);
        this._reset_renderer_caches();
    }

    /**
     * The game's state as a portable state (see docs/portable-state.md): a
     * fraction of the size of [`save_state`], independent of this build's
     * addresses, and loadable by the headless build (Rust, Python, nodes) as
     * well as by this one. Throws if the game is in a state with no portable
     * form (in the intro while the Mario head runs).
     */
    export_portable_state() {
        return this._translator().exportState(this.memory.buffer, BUILD_VISUAL);
    }

    /**
     * Makes this game the one a portable state was exported from, by either
     * build (`export_portable_state` here, or `SM64Game::export_portable` in
     * Rust). Returns {blocks allocated, pointers without a translation}.
     */
    load_portable_state(bytes) {
        const t = this._translator();
        const info = portableInfo(bytes);
        // Load into the game as booted, not into one that has played: the translation keeps the
        // target's heap and allocates the state's blocks in it, and a heap holding a level's
        // data ends in a corrupted heap (abort() a few frames later, or textures drawn black).
        // Restoring the boot snapshot is a plain copy of this game's own memory.
        if (this._bootState) this.load_state(this._bootState);
        const exports = this.wasm.exports;
        const r = t.loadState(bytes, this.memory.buffer, BUILD_VISUAL, (n) => exports.malloc(n));
        const mem = new Uint8Array(this.memory.buffer);
        for (const p of r.patches) mem.set(p.bytes, p.addr);
        // The pool the display lists came from lost its list of blocks.
        for (const a of r.stale) exports.free(a);
        this._reset_renderer_caches();
        return { source: info.source, allocated: r.allocated, untranslated: r.untranslated, ghostBytes: r.ghostBytes };
    }

    // The renderer caches GL textures by the address of the texture's data. A
    // loaded state can have other data at a cached address, so start the cache
    // over: its nodes keep their GL texture ids (a node is reused as it is
    // needed again), only the lookup table and the number in use reset.
    _reset_renderer_caches() {
        const c = this.translator.constants(BUILD_VISUAL);
        const mem = new Uint8Array(this.memory.buffer);
        mem.fill(0, c.textureCache, c.textureCache + 4096);
        mem.fill(0, c.textureCache + c.textureCacheSize - 4, c.textureCache + c.textureCacheSize);
    }
}

export { Sm64VisualEngine };
