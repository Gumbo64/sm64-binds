// Portable save states in the browser.
//
// The headless build (what Rust, Python and the nodes run) and the rendering
// build (what Sm64VisualEngine runs) lay their memory out differently, so a
// state from one cannot be loaded as bytes in the other. A *portable* state
// (format "SM64PORT", see docs/portable-state.md) holds the game's variables
// and malloc blocks with every pointer made build-neutral. The translation is
// the Rust code in rust/src/sm64/portable, compiled to a small WASM module
// (rust/portable-wasm, shipped as pkg/portable_wasm.js).

import { PORTABLE_WASM_BASE64 } from "./pkg/portable_wasm.js";

export const BUILD_HEADLESS = 0;
export const BUILD_VISUAL = 1;

const MAGIC = "SM64PORT";

function base64ToBytes(b64) {
    if (typeof atob === "function") {
        const s = atob(b64);
        const out = new Uint8Array(s.length);
        for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
        return out;
    }
    return new Uint8Array(Buffer.from(b64, "base64"));
}

let translatorPromise = null;

/** The translator WASM: exports a game's state as a portable one, and works out
 *  what loading a portable state into a game's memory has to write. */
export class PortableTranslator {
    constructor(instance) {
        this.x = instance.exports;
    }

    /** Loads the module once. */
    static load() {
        translatorPromise ??= WebAssembly.instantiate(base64ToBytes(PORTABLE_WASM_BASE64), {}).then(
            (r) => new PortableTranslator(r.instance),
        );
        return translatorPromise;
    }

    _mem() {
        return new Uint8Array(this.x.memory.buffer);
    }

    _put(bytes) {
        const ptr = this.x.p_alloc(bytes.length);
        this._mem().set(bytes, ptr); // after p_alloc: it may have grown the memory
        return ptr;
    }

    _error() {
        const e = new TextDecoder().decode(this._mem().subarray(this.x.p_error_ptr(), this.x.p_error_ptr() + this.x.p_error_len()));
        return new Error(e);
    }

    /** The build's constants: the allocator state's address (`gm`), where its
     *  constant data ends (`dataStart`), and the rendering build's texture cache. */
    constants(build) {
        const x = this.x;
        return {
            gm: x.p_const(0, build),
            dataStart: x.p_const(1, build),
            textureCache: x.p_const(2, 1),
            textureCacheSize: x.p_const(3, 1),
            geoCacheGeneration: x.p_const(4, 0),
            geoCacheCount: x.p_const(5, 0),
        };
    }

    /** The build id (the SHA-1 of its WASM's code, hex) these tables are for. */
    buildId(build) {
        const p = this.x.p_build_id(build);
        return new TextDecoder().decode(this._mem().subarray(p, p + 40));
    }

    /** How much of a `build` game's memory a translation reads: up to the top
     *  of its malloc heap. */
    memoryLength(buffer, build) {
        const { gm } = this.constants(build);
        const top = new DataView(buffer).getUint32(gm + 24, true);
        return Math.min(buffer.byteLength, top + 0x1000);
    }

    /** The state of the game whose linear memory is `buffer` (an ArrayBuffer of
     *  a `build` game), as portable bytes. Throws if the state has no portable
     *  form (such as in the intro, while the Mario head runs). */
    exportState(buffer, build) {
        const len = this.memoryLength(buffer, build);
        const x = this.x;
        const mem = x.p_alloc(len);
        this._mem().set(new Uint8Array(buffer, 0, len), mem);
        const n = x.p_export(mem, len, build);
        x.p_free(mem, len);
        if (n < 0) throw this._error();
        const out = this._mem().slice(x.p_result_ptr(), x.p_result_ptr() + n);
        x.p_clear();
        return out;
    }

    /** One 64-bit hash (as a BigInt) per variable of the state: for checking
     *  that two games (of either build) are in the same state. See the Rust
     *  `Portable::digest`. */
    digestState(buffer, build) {
        const len = this.memoryLength(buffer, build);
        const x = this.x;
        const mem = x.p_alloc(len);
        this._mem().set(new Uint8Array(buffer, 0, len), mem);
        const n = x.p_digest(mem, len, build);
        x.p_free(mem, len);
        if (n < 0) throw this._error();
        const out = new BigUint64Array(this._mem().slice(x.p_result_ptr(), x.p_result_ptr() + n).buffer);
        x.p_clear();
        return out;
    }

    /** Loads portable `state` into the `target` game whose linear memory is
     *  `buffer`, allocating with `malloc(size)` (the game's own). Returns what it
     *  did, including `stale`: blocks of the target the caller must free. */
    loadState(state, buffer, target, malloc) {
        const x = this.x;
        const len = this.memoryLength(buffer, target);
        const mem = x.p_alloc(len);
        this._mem().set(new Uint8Array(buffer, 0, len), mem);
        const st = this._put(state);
        const need = x.p_load_begin(st, state.length, mem, len, target);
        x.p_free(mem, len);
        x.p_free(st, state.length);
        if (need < 0) throw this._error();
        const stale = [];
        for (let i = 0; i < x.p_stale_count(); i++) stale.push(x.p_stale(i));
        for (let i = 0; i < need; i++) {
            const size = x.p_need_size(i);
            const addr = malloc(size) >>> 0;
            if (addr === 0) throw new Error("malloc failed while loading a portable state");
            x.p_set_alloc(i, addr);
        }
        const n = x.p_load_finish(target);
        if (n < 0) throw this._error();
        const patches = [];
        for (let i = 0; i < n; i++) {
            const p = x.p_patch_ptr(i);
            patches.push({ addr: x.p_patch_addr(i), bytes: this._mem().slice(p, p + x.p_patch_len(i)) });
        }
        const untranslated = x.p_untranslated();
        x.p_clear();
        return { patches, stale, untranslated, allocated: need };
    }
}

/** What a portable state says about itself (without the tables): the build it
 *  was exported from, and sizes. Throws if `bytes` is not one. */
export function portableInfo(bytes) {
    const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
    if (bytes.length < 48 || String.fromCharCode(...bytes.subarray(0, 8)) !== MAGIC) {
        throw new Error("not a portable sm64 state");
    }
    return {
        version: dv.getUint32(8, true),
        source: dv.getUint32(12, true) === 0 ? "headless" : "visual",
        buildId: Array.from(bytes.subarray(16, 36), (b) => b.toString(16).padStart(2, "0")).join(""),
        variables: dv.getUint32(40, true),
        blocks: dv.getUint32(44, true),
        size: bytes.length,
    };
}

const LEB_DATA_SECTIONS = new Set([11, 12]);

/** The SHA-1 (hex) of a WASM's code: the module without its data sections,
 *  which is how the build is identified in save states (the same as the Rust
 *  crate's build id). */
export async function wasmCodeId(wasm) {
    const b = new Uint8Array(wasm);
    const parts = [b.subarray(0, 8)];
    let at = 8;
    while (at < b.length) {
        const id = b[at];
        let p = at + 1;
        let size = 0;
        for (let shift = 0; ; shift += 7) {
            const byte = b[p++];
            size += (byte & 0x7f) * 2 ** shift;
            if (byte < 0x80) break;
        }
        const end = p + size;
        if (!LEB_DATA_SECTIONS.has(id)) parts.push(b.subarray(at, end));
        at = end;
    }
    const all = new Uint8Array(parts.reduce((n, x) => n + x.length, 0));
    let o = 0;
    for (const x of parts) {
        all.set(x, o);
        o += x.length;
    }
    const h = new Uint8Array(await crypto.subtle.digest("SHA-1", all));
    return Array.from(h, (x) => x.toString(16).padStart(2, "0")).join("");
}
