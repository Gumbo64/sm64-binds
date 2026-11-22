// The headless build's WASM for the browser: the same payload as the Rust and
// Python packages (XORed with the ROM, zipped), decoded with the ROM the user
// gave and cached in IndexedDB next to the rendering build. Kept apart from
// fileUpload.js, and only imported by the worker, so the ~6 MB payload is
// downloaded only when a replay needs the headless catch-up.

import headlessXor from './sm64/pkg/sm64_headless.us.wasm.zip.xor.png';
import { getFile, storeFile } from './fileUpload.js';
import { decodeHeadlessWasm } from './headlessDecode.js';

const NAME = 'sm64_headless.us.wasm';

/** The headless WASM's bytes (an ArrayBuffer). Needs the ROM stored (storeROM). */
export async function getHeadlessWASM() {
    try {
        return await getFile(NAME);
    } catch {
        // not cached yet
    }
    const rom = await getFile('ROM');
    const res = await fetch(headlessXor, { credentials: 'same-origin' });
    if (!res.ok) throw new Error(`HTTP error ${res.status} fetching the headless game`);
    const wasm = await decodeHeadlessWasm(await res.arrayBuffer(), rom);
    await storeFile(NAME, new Uint8Array(wasm));
    return wasm;
}
