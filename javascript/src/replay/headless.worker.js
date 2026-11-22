// Web Worker: the headless game (no canvas, GL or audio), for catching a
// replay up. Steps the movie's pads to a frame as fast as it can and posts
// the game as a portable state, which the page loads into the rendering
// engine; see headless.js (the page's side) for the messages.
//
// The game stays where a job left it, so the next job continues from there
// when it can, else starts from power-on. Jobs run in slices and yield, so a
// newer job replaces the running one between slices.

import { Sm64HeadlessEngine } from '../scripts/sm64/Sm64Headless.js';
import { getHeadlessWASM } from '../scripts/headlessWasm.js'; // (the worker is itself the lazy part)

const SLICE_MS = 40;

let engine = null;
let pads = null; // flat Int32Array [button, stick_x, stick_y, ...]
let at = -1; // the frame the game is at (before its pad), -1 when unknown
let current = 0; // the id of the job that may run

const channel = new MessageChannel();
const waiting = [];
channel.port1.onmessage = () => waiting.shift()?.();
const yieldTask = () => new Promise((r) => (waiting.push(r), channel.port2.postMessage(0)));

function post(msg, transfer = []) {
  self.postMessage(msg, transfer);
}

async function init() {
  engine = await Sm64HeadlessEngine.create(await getHeadlessWASM());
  at = 0;
  post({ type: 'ready', codeId: engine.codeId });
}

/**
 * Where a job starts: the game as it is when it has not passed the target (nor
 * `maxStart`, the page's limit for events to stay complete), else power-on. Never
 * a portable state loaded into the game: the game here is always the continuous
 * run of the movie, which is exact, so the only translation a landing goes
 * through is the one into the rendering engine.
 */
function startFrom(job) {
  const { target, maxStart } = job;
  if (at >= 0 && at <= target && at <= maxStart) return at;
  engine.restart();
  at = 0;
  return 0;
}

async function run(job) {
  const { id, target, every, skip } = job;
  const t0 = performance.now();
  const start = startFrom(job);
  const skipSet = new Set(skip ?? []);
  const events = [];
  const states = [];
  let prev = job.events ? engine.progress() : null;
  let lastPost = performance.now();
  let steps = 0;
  while (at < target) {
    const sliceEnd = performance.now() + SLICE_MS;
    while (at < target && performance.now() < sliceEnd) {
      if (every && at % every === 0 && at > start && !skipSet.has(at)) {
        try {
          states.push([at, engine.export_portable_state()]);
        } catch {
          // no portable state here (the intro): the page's trail has a gap
        }
      }
      engine.step_flat(pads, at);
      at += 1;
      steps += 1;
      if (prev) {
        const s = engine.progress();
        if (s[0] > prev[0]) events.push({ frame: at, kind: 'star', label: `Star ${s[0]}` });
        if (prev[2] !== 0 && (s[1] !== prev[1] || s[2] !== prev[2])) events.push({ frame: at, kind: 'area', course: s[1], area: s[2] });
        prev = s;
      }
    }
    if (current !== id) return;
    if (at < target) {
      if (performance.now() - lastPost > 100 || states.length) {
        lastPost = performance.now();
        const batch = states.splice(0);
        post({ type: 'progress', id, frame: at, states: batch }, batch.map((s) => s[1].buffer));
      }
      await yieldTask();
      if (current !== id) return;
    }
  }
  let state = null;
  if (job.land) state = engine.export_portable_state(); // throws where there is none
  const gs = engine.get_game_state();
  const batch = states.splice(0);
  const transfer = batch.map((s) => s[1].buffer);
  if (state) transfer.push(state.buffer);
  post(
    {
      type: 'done',
      id,
      frame: at,
      start,
      state,
      states: batch,
      events,
      steps,
      ms: performance.now() - t0,
      game: { pos: gs.pos, stars: gs.numStars, action: engine.mario_action() },
    },
    transfer,
  );
}

self.onmessage = async (e) => {
  const m = e.data;
  try {
    if (m.type === 'init') {
      pads = m.pads;
      await init();
    } else if (m.type === 'goto' || m.type === 'fill') {
      current = m.id;
      if (!engine) throw new Error('the headless game is not ready');
      await run(m);
    } else if (m.type === 'stop') {
      current = 0;
    }
  } catch (err) {
    post({ type: 'error', id: m.id ?? 0, message: String(err?.message ?? err) });
  }
};
