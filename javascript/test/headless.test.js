// The headless catch-up: the Player's use of a worker (against a fake worker
// and a fake game), and, when the ROM is there (SM64_ROM, or ../baserom.us.z64),
// the headless game itself (Sm64Headless.js) in Node.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import crypto from 'node:crypto';
import { Player } from '../src/replay/player.js';
import { flattenPads } from '../src/replay/headless.js';

const u32 = (n) => new Uint8Array(new Uint32Array([n]).buffer);
const num = (b) => new Uint32Array(b.buffer, b.byteOffset, 1)[0];

function fakeEngine() {
  const e = {
    n: 0,
    portable_supported: true,
    step_game() { e.n++; },
    set_audio_enabled() {},
    get_game_state() { return { numStars: 0, courseNum: 0, areaIndex: 1 }; },
    export_portable_state() { return u32(e.n); },
    load_portable_state(b) { e.n = num(b); },
  };
  return e;
}

/** A worker that follows headless.worker.js's protocol, with the game faked as a frame counter. */
function fakeWorkerFactory(log, { failGoto = false, failInit = false, delay = 1 } = {}) {
  return () => {
    const w = {
      postMessage(m) {
        log.push(m);
        const reply = (r, transfer) => setTimeout(() => w.onmessage({ data: r }), delay);
        if (m.type === 'init') {
          if (failInit) setTimeout(() => w.onerror({ message: 'no wasm' }), delay);
          else reply({ type: 'ready', codeId: 'fake' });
        } else if (m.type === 'goto') {
          if (failGoto) return reply({ type: 'error', id: m.id, message: 'no portable state here' });
          const states = [];
          for (let f = m.every; f > 0 && f < m.target; f += m.every) if (f > (m.base?.frame ?? 0)) states.push([f, u32(f)]);
          reply({ type: 'done', id: m.id, frame: m.target, start: m.base?.frame ?? 0, state: u32(m.target), states, events: [{ frame: 5, kind: 'star', label: 'Star 1' }], steps: m.target, ms: 1, game: {} });
        } else if (m.type === 'fill') {
          const states = [];
          for (let f = m.every; f < m.target; f += m.every) states.push([f, u32(f)]);
          reply({ type: 'done', id: m.id, frame: m.target, start: 0, state: null, states, events: [], steps: m.target, ms: 1, game: {} });
        }
      },
      terminate() { w.dead = true; },
    };
    return w;
  };
}

function player(log, opts = {}, workerOpts = {}, n = 2000) {
  const pads = Array.from({ length: n }, () => ({ button: 0, stick_x: 0, stick_y: 0 }));
  const p = new Player(pads, null, { makeHeadlessWorker: fakeWorkerFactory(log, workerOpts), ...opts });
  p.engine = fakeEngine();
  p.canvas = { getContext: () => null };
  p.trail.every = 400;
  p.trail.add(0, u32(0));
  return p;
}

const settle = async (p) => {
  for (let i = 0; i < 400 && p.seeking; i++) await new Promise((r) => setTimeout(r, 5));
  assert.ok(!p.seeking, 'the seek finished');
};
const jobs = (log) => log.filter((m) => m.type === 'goto');

test('flattenPads packs the pads for the worker', () => {
  const f = flattenPads([{ button: 0x8000, stick_x: -5, stick_y: 7 }, { button: 1, stick_x: 0, stick_y: 0 }]);
  assert.deepEqual([...f], [0x8000, -5, 7, 1, 0, 0]);
});

test('a long seek is caught up by the worker, which lands one frame short; the engine plays the last', async () => {
  const log = [];
  const p = player(log);
  assert.equal(p.seek(1500), true);
  assert.ok(p.seeking);
  await settle(p);
  assert.equal(jobs(log).length, 1);
  assert.equal(jobs(log)[0].target, 1499);
  assert.equal(p.frame, 1500);
  assert.equal(p.engine.n, 1500);
  assert.equal(p.lastHeadless.landed, 1499);
  await new Promise((r) => setTimeout(r, 30));
  assert.ok(p.trail.has(400) && p.trail.has(800), 'the worker filled the trail afterwards');
  assert.ok(p.events.some((e) => e.frame === 5), 'and the events');
});

test('a short seek keeps using the trail', async () => {
  const log = [];
  const p = player(log);
  p.seek(500);
  await settle(p);
  assert.equal(jobs(log).length, 0);
  assert.equal(p.frame, 500);
  assert.equal(p.engine.n, 500);
});

test('the worker is not used when it is turned off', async () => {
  const log = [];
  const p = player(log, { headless: false });
  p.seek(1500);
  await settle(p);
  assert.equal(log.length, 0);
  assert.equal(p.frame, 1500);
});

test('headlessLimit: frames after it are played by the engine (a perturbed block, say)', async () => {
  const log = [];
  const p = player(log);
  p.headlessLimit = () => 1000;
  p.workerTrail = false;
  p.seek(1800);
  await settle(p);
  assert.equal(jobs(log)[0].target, 1000);
  assert.equal(log.filter((m) => m.type === 'fill').length, 0, 'no fill pass for a player whose states are not portable');
  assert.equal(p.engine.n, 1800);
  assert.equal(p.frame, 1800);
  assert.ok(!p.trail.has(800), 'its states are not the worker\'s portable ones');
});

test('a seek inside a stretch the trail already covers does not use the worker', async () => {
  const log = [];
  const p = player(log);
  p.headlessLimit = () => 1000;
  p.trail.add(1700, u32(1700));
  p.seek(1800);
  await settle(p);
  assert.equal(jobs(log).length, 0);
  assert.equal(p.engine.n, 1800);
});

test('a worker that fails falls back to the rendering build, and says so', async () => {
  const log = [];
  const p = player(log, {}, { failGoto: true });
  const warnings = [];
  const warn = console.warn;
  console.warn = (...a) => warnings.push(a.join(' '));
  try {
    p.seek(1500);
    await settle(p);
  } finally {
    console.warn = warn;
  }
  assert.equal(p.frame, 1500);
  assert.equal(p.engine.n, 1500);
  assert.ok(warnings.some((w) => /headless catch-up failed/.test(w)));
});

test('a worker that does not start falls back too, and is not tried again', async () => {
  const log = [];
  const p = player(log, {}, { failInit: true });
  const warn = console.warn;
  const warnings = [];
  console.warn = (...a) => warnings.push(a.join(' '));
  try {
    p.seek(1500);
    await settle(p);
    assert.equal(p.engine.n, 1500);
    assert.ok(p.headlessFailed);
    p.seek(0);
    await settle(p);
    p.seek(1500);
    await settle(p);
  } finally {
    console.warn = warn;
  }
  assert.equal(p.engine.n, 1500);
  assert.equal(log.filter((m) => m.type === 'init').length, 1);
  assert.ok(warnings.length >= 1);
});

test('a newer seek replaces a catch-up under way', async () => {
  const log = [];
  const p = player(log, {}, { delay: 30 });
  p.seek(1900);
  p.seek(40); // short: the trail
  await settle(p);
  await new Promise((r) => setTimeout(r, 100));
  assert.equal(p.frame, 40);
  assert.equal(p.engine.n, 40);
});

test('a backwards seek far from any state goes through the worker too', async () => {
  const log = [];
  const p = player(log);
  p.trail.states.delete(0);
  p.trail.add(0, u32(0));
  p.seek(1500);
  await settle(p);
  p.trail.states.clear(); // as if the states were lost
  p.trail.add(0, u32(0));
  p.seek(1200);
  await settle(p);
  assert.equal(jobs(log).length, 2);
  assert.equal(p.frame, 1200);
  assert.equal(p.engine.n, 1200);
});

// --- the headless game itself, with the ROM ---

const rom = [process.env.SM64_ROM, new URL('../../baserom.us.z64', import.meta.url).pathname].find((f) => f && fs.existsSync(f));
const payload = new URL('../../rust/pkg/sm64_headless.us.wasm.zip.xor', import.meta.url).pathname;
const movie = new URL('../../tas/tasvideos-7310M-120star.m64', import.meta.url).pathname;
const have = !!rom && fs.existsSync(payload) && fs.existsSync(movie);

test('the headless game runs in JavaScript, and its portable state round-trips', { skip: !have && 'needs the ROM (SM64_ROM or ../baserom.us.z64)' }, async () => {
  const { decodeHeadlessWasm } = await import('../src/scripts/headlessDecode.js');
  const { Sm64HeadlessEngine } = await import('../src/scripts/sm64/Sm64Headless.js');
  const { parseReplay } = await import('../src/replay/m64.js');
  const wasm = await decodeHeadlessWasm(fs.readFileSync(payload), fs.readFileSync(rom));
  // Consensus: the build must not change.
  assert.equal(crypto.createHash('sha1').update(Buffer.from(wasm)).digest('hex'), 'a6a197b5ec48fa11725d009eed4af37fbacd00c0');
  const { pads } = parseReplay(fs.readFileSync(movie));
  const flat = flattenPads(pads);
  const e = await Sm64HeadlessEngine.create(wasm);
  const run = (g, from, to) => { for (let i = from; i < to; i++) g.step_flat(flat, i); };
  const snap = (g) => JSON.stringify([g.get_game_state().toString(), g.mario_action()]);
  run(e, 0, 3000);
  const state = e.export_portable_state();
  assert.equal(String.fromCharCode(...state.subarray(0, 8)), 'SM64PORT');
  run(e, 3000, 3200);
  const want = snap(e);
  // A fresh game that loads the state plays on identically.
  const f = await Sm64HeadlessEngine.create(e.module);
  f.load_portable_state(state);
  run(f, 3000, 3200);
  assert.equal(snap(f), want);
  // restart() is power-on again.
  e.restart();
  run(e, 0, 3200);
  assert.equal(snap(e), want);
  assert.deepEqual(e.progress().length, 3);
});
