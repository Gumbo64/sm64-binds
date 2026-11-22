// ROM-XOR and unzip of the headless payload (no browser or bundler imports,
// so it also runs under Node).
import JSZip from 'jszip';
import { xorBytes } from './xor.js';

/** `xored`: the contents of sm64_headless.us.wasm.zip.xor; `rom`: the ROM. Returns the WASM as an ArrayBuffer. */
export async function decodeHeadlessWasm(xored, rom) {
    const zipBytes = xorBytes(new Uint8Array(xored), new Uint8Array(rom));
    let zip;
    try {
        zip = await new JSZip().loadAsync(zipBytes);
    } catch (e) {
        throw new Error(`the headless game did not decode (wrong ROM?): ${e.message ?? e}`);
    }
    const file = Object.values(zip.files)[0];
    if (!file) throw new Error('the headless game payload is empty');
    return file.async('arraybuffer');
}
