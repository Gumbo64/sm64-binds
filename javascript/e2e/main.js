// The page the portable state checks (portable_check.mjs) drive in Chromium.
import { storeROM } from '@fe/src/scripts/fileUpload.js';
import { Sm64VisualEngine, GamePad } from '@fe/src/scripts/sm64/Sm64Game.js';

window.engine = null;

window.vInit = async function () {
  if (!window.__rom) {
    const buf = await (await fetch('/rom.z64')).arrayBuffer();
    await storeROM(buf);
    window.__rom = true;
  }
  window.engine = await Sm64VisualEngine.create(document.getElementById('canvas'));
  window.engine.set_audio_enabled(0);
  return window.engine.portable_supported;
};

function toB64(bytes) {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}
function fromB64(b64) {
  const s = atob(b64);
  const out = new Uint8Array(s.length);
  for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
  return out;
}

// flat pads: [button, stick_x, stick_y, ...]
window.vStep = function (flat) {
  for (let i = 0; i < flat.length; i += 3) window.engine.step_game(new GamePad(flat[i], flat[i + 1], flat[i + 2]));
};

// The GameState in the order the Rust side records it.
function gs() {
  const s = window.engine.get_game_state();
  return [s.numStars, ...s.pos, ...s.vel, ...s.lakituPos, s.lakituYaw, s.inCredits, s.courseNum, s.actNum, s.areaIndex, s.levelNum, s.lives, s.coins, s.health];
}

// Steps through `flat` (K pads), recording after every frame.
window.vRecord = function (flat) {
  const out = [];
  for (let i = 0; i < flat.length; i += 3) {
    window.vStep(flat.slice(i, i + 3));
    let d = '';
    try {
      d = toB64(new Uint8Array(window.engine.translator.digestState(window.engine.memory.buffer, 1).buffer));
    } catch (e) {
      d = '';
    }
    out.push({ gs: gs(), d });
  }
  return out;
};

window.vExport = function () {
  return toB64(window.engine.export_portable_state());
};

window.vLoad = function (b64) {
  return window.engine.load_portable_state(fromB64(b64));
};

// Own-build snapshots: save, play on, load, play on again: same?
window.vSnapshot = function () {
  window.__snap = window.engine.save_state();
  return window.__snap.length;
};
window.vRestore = function () {
  window.engine.load_state(window.__snap);
};
