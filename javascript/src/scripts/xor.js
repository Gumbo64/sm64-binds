// Pure byte helpers, kept free of browser and asset imports so they can be
// unit tested under `node --test`.

/**
 * XORs `data` with `key`, repeating the key as needed.
 *
 * Walks the key in chunks rather than indexing it with `i % keyLength` per
 * byte, which is worth ~1.7x on the 6 MB payload this repo ships.
 */
export function xorBytes(data, key) {
    const keyLength = key.byteLength;
    if (keyLength === 0) {
        throw new Error("XOR key must not be empty");
    }
    const out = new Uint8Array(data.length);
    for (let offset = 0; offset < data.length; offset += keyLength) {
        const chunk = Math.min(keyLength, data.length - offset);
        for (let i = 0; i < chunk; i++) {
            out[offset + i] = data[offset + i] ^ key[i];
        }
    }
    return out;
}