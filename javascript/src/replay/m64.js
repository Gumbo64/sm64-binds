// Replay files.
//
// Two layouts are accepted:
//  - raw: 4 bytes per frame (big-endian u16 buttons, i8 stick x, i8 stick y),
//    as written by sm64-AI-binds;
//  - Mupen64 .m64: the same frames after a 0x400-byte header starting with
//    "M64\x1A".

const M64_MAGIC = [0x4d, 0x36, 0x34, 0x1a];
const M64_HEADER = 0x400;

export function hasMupenHeader(bytes) {
  return bytes.length >= M64_HEADER && M64_MAGIC.every((b, i) => bytes[i] === b);
}

/** Parses replay bytes into `{ button, stick_x, stick_y }` frames. */
export function parseReplay(buffer) {
  let bytes = new Uint8Array(buffer);
  const header = hasMupenHeader(bytes);
  // The header's input count (at 0x18): files can have padding after it.
  const count = header ? new DataView(bytes.buffer, bytes.byteOffset).getUint32(0x18, true) : Infinity;
  if (header) bytes = bytes.subarray(M64_HEADER);
  const n = Math.min(count, Math.floor(bytes.length / 4));
  const pads = new Array(n);
  for (let i = 0; i < n; i++) {
    const o = i * 4;
    pads[i] = {
      button: (bytes[o] << 8) | bytes[o + 1],
      stick_x: (bytes[o + 2] << 24) >> 24,
      stick_y: (bytes[o + 3] << 24) >> 24,
    };
  }
  return { pads, header, trailing: bytes.length % 4 };
}

export const FPS = 30;

export function formatTime(frame) {
  const s = frame / FPS;
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return `${m}:${rest.toFixed(2).padStart(5, '0')}`;
}
