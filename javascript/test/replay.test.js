// The replay player's non-UI parts: movie parsing, regions, the Trail, and the
// Player's seeking through engine save states (against a fake engine).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { parseReplay, formatTime, FPS } from '../src/replay/m64.js';
import { normalizeRegions, regionAt } from '../src/replay/regions.js';
import { Player } from '../src/replay/player.js';

const pkg = JSON.parse(fs.readFileSync(new URL('../package.json', import.meta.url), 'utf8'));

test('package exposes the replay player as a subpath, with React as a peer', () => {
  assert.equal(pkg.exports['./replay'], './src/replay/index.js');
  assert.equal(pkg.exports['./replay.css'], './src/replay/replay.css');
  assert.ok(pkg.peerDependencies.react && pkg.peerDependencies['react-dom']);
  assert.ok(!pkg.dependencies.react && pkg.dependencies['mp4-muxer']);
  for (const f of ['index.js', 'replay.css', 'Viewer.jsx']) {
    assert.ok(fs.existsSync(new URL(`../src/replay/${f}`, import.meta.url)), f);
  }
  const idx = fs.readFileSync(new URL('../src/replay/index.js', import.meta.url), 'utf8');
  for (const n of ['Viewer', 'Player', 'SPEEDS', 'Trail', 'parseReplay', 'formatTime', 'normalizeRegions', 'regionAt']) {
    assert.ok(new RegExp(`\\b${n}\\b`).test(idx), n);
  }
});

test('parseReplay reads raw and Mupen movies', () => {
  const raw = Uint8Array.from([0x80, 0x00, 5, 0xfb, 0x00, 0x01, 0, 0]);
  const r = parseReplay(raw.buffer);
  assert.equal(r.header, false);
  assert.deepEqual(r.pads, [
    { button: 0x8000, stick_x: 5, stick_y: -5 },
    { button: 1, stick_x: 0, stick_y: 0 },
  ]);
  const m64 = new Uint8Array(0x400 + 8);
  m64.set([0x4d, 0x36, 0x34, 0x1a]);
  new DataView(m64.buffer).setUint32(0x18, 1, true);
  m64.set([0x10, 0x00, 1, 2], 0x400);
  const p = parseReplay(m64.buffer);
  assert.equal(p.header, true);
  assert.equal(p.pads.length, 1);
  assert.equal(p.pads[0].button, 0x1000);
});

test('formatTime and regions', () => {
  assert.equal(FPS, 30);
  assert.equal(formatTime(30 * 65), '1:05.00');
  const rs = normalizeRegions([{ from: 100, label: 'b' }, { from: 0, label: 'a' }, { label: 'x' }], 300);
  assert.deepEqual(rs.map((r) => [r.from, r.to, r.label]), [[0, 100, 'a'], [100, 300, 'b']]);
  assert.equal(regionAt(rs, 99).label, 'a');
  assert.equal(regionAt(rs, 100).label, 'b');
  assert.equal(regionAt(rs, 300), null);
});

// A game that counts frames; its state is the count.
function fakeEngine({ portable }) {
  const e = {
    n: 0,
    portable_supported: portable,
    step_game() { e.n++; },
    set_audio_enabled() {},
    get_game_state() { return { numStars: 0, courseNum: 0, areaIndex: 1 }; },
    export_portable_state() { return Uint8Array.of(e.n); },
    load_portable_state(b) { e.n = b[0]; },
    save_state() { return Uint8Array.of(e.n); },
    save_state_delta(base) { return Uint8Array.of(e.n, base.length); },
    load_state(b) { e.n = b[0]; },
  };
  return e;
}

for (const portable of [true, false]) {
  test(`Player seeks backwards through ${portable ? 'portable' : 'raw delta'} states`, () => {
    const pads = Array.from({ length: 1000 }, () => ({ button: 0, stick_x: 0, stick_y: 0 }));
    const p = new Player(pads, null);
    p.engine = fakeEngine({ portable });
    p.canvas = { getContext: () => null };
    p.trail.every = 100;
    p.trail.add(0, p.saveState());
    for (let i = 0; i < 450; i++) p.advance();
    assert.equal(p.frame, 450);
    assert.equal(p.engine.n, 450);
    assert.ok(p.trail.has(400));
    assert.equal(p.seek(130), true); // back to the state at 100, then forwards
    assert.equal(p.frame, 130); // the fast-forward ran to the target
    assert.equal(p.engine.n, 130);
    assert.equal(p.seekTarget, null);
  });
}
