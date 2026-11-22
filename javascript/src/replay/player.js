// Drives the emulator: real-time playback at any speed, single-frame steps,
// fast-forward seeking and event detection. Backwards seeking loads the
// nearest earlier state of a Trail of engine save states; if there is none
// (the intro, where no state can be taken) seek() returns false and the app
// starts a fresh emulator (typically by reloading).

import { FPS } from './m64.js';
import { ResolutionControl, loadResolution } from './resolution.js';
import { Trail } from './trail.js';
import { HeadlessClient, headlessSupported } from './headless.js';

export const SPEEDS = [0.25, 0.5, 1, 2, 4, 8, 16, Infinity];

// Wall-clock budget per animation frame while fast-forwarding / playing at
// max speed, so the page stays responsive.
const BUDGET_MS = 14;
// A seek works in slices this long between yields to the page (nothing is
// drawn or heard while it runs, so a long slice is fine).
const SEEK_SLICE_MS = 40;
// While playing, the background trail pass gets one hop this often.
const HOP_GAP_MS = 700;
// A seek goes through the headless worker when that saves more than this many
// frames of stepping the rendering build (see Player's `headless` option).
const HEADLESS_THRESHOLD = 600;
// Replays at least this long start the worker as soon as they are attached,
// to fill the trail; shorter ones start it only when a long seek needs it.
const HEADLESS_FILL_MIN = 1500;

// A message channel yields to the page without the 4 ms clamp of setTimeout
// and without waiting for an animation frame (which a busy page delays).
const channel = typeof MessageChannel !== 'undefined' ? new MessageChannel() : null;
const waiting = [];
if (channel) {
  channel.port1.onmessage = () => waiting.shift()?.();
  channel.port1.unref?.(); // (Node) don't keep a process alive
}
const yieldTask = () => new Promise((r) => (channel ? (waiting.push(r), channel.port2.postMessage(0)) : setTimeout(r, 0)));

// Skips the GL draw calls (state and textures still update, only pixels are
// not drawn): what makes seeking and the trail pass fast. Returns the undo.
function muteDrawing(canvas) {
  let gl = null;
  try {
    gl = canvas.getContext('webgl2') || canvas.getContext('webgl');
  } catch {
    // no context
  }
  if (!gl) return () => {};
  const saved = {};
  for (const k of ['drawElements', 'drawArrays', 'clear']) {
    saved[k] = gl[k];
    gl[k] = () => {};
  }
  return () => {
    for (const k in saved) gl[k] = saved[k];
  };
}

export class Player {
  /**
   * `options.headless` (default true): catch up and fill the trail with the
   * headless game in a Web Worker (see headless.js) instead of stepping the
   * rendering build; `headlessThreshold`: how many frames a seek has to save
   * before the worker is used; `makeHeadlessWorker`: how to create the worker
   * when the bundler does not understand `new Worker(new URL(...))`.
   */
  constructor(pads, onChange, options = {}) {
    this.pads = pads;
    this.onChange = onChange;
    this.options = { headless: true, headlessThreshold: HEADLESS_THRESHOLD, makeHeadlessWorker: null, ...options };
    this.headless = null; // HeadlessClient, made when first wanted
    this.headlessReady = false;
    this.headlessFailed = !this.options.headless || !headlessSupported(this.options.makeHeadlessWorker);
    this.headlessBusy = false; // a worker catch-up is under way
    this.headlessToken = 0; // bumped by every seek: a catch-up that finds it changed has been replaced
    this.headlessProgress = null; // the frame the worker has got to (shown while catching up)
    this.headlessErrors = 0;
    this.pristine = null; // the engine's own snapshot at power-on
    this.workerTrail = true; // the worker's states may go into the trail (a subclass with other states says false)
    this.fillRunning = false;
    this.fillDone = false;
    this.evThrough = 0; // `events` is complete up to this frame
    this.lastHeadless = null; // what the last catch-up did ({from, to, steps, ms, workerMs, ...}), for measuring
    this.engine = null;
    this.canvas = null;
    this.resolution = null;
    this.frozen = null; // copy of the last rendered frame, see capture()
    this.rendered = false;
    this.frame = 0; // frames already played
    this.playing = false;
    this.speed = 1;
    this.seekTarget = null;
    this.state = null;
    this.events = [];
    this.audio = null;
    this.raf = 0;
    this.lastTime = null;
    this.debt = 0;
    this.dirty = true;
    // Save states, for seeking in both directions (see saveState / loadState):
    // one every `every` frames, at most about 300 whatever the length.
    this.trail = new Trail(Math.max(240, Math.ceil(pads.length / 300)), 400);
    this.trailSkipped = new Set(); // frames the background pass could not take a state at
    this.quiet = false; // stepping without drawing (a seek or a trail hop)
    this.trackEvents = true; // read the game state on every frame, even while quiet (for the events list)
    this.seekRunning = false;
    this.hopping = false;
    this.lastHop = 0;
    this.dead = false;
    this.rawBase = null; // the first raw snapshot, which raw deltas are taken against
    this.exporting = false; // exporter.js is stepping the game; tick() stays out of it
  }

  /** Creates the emulator on a fresh canvas inside `container`. */
  async attach(container) {
    const canvas = document.createElement('canvas');
    canvas.className = 'game-canvas';
    container.replaceChildren(canvas);
    // The Emscripten build looks the canvas up by id, at startup and again
    // whenever it resizes, so the id stays.
    canvas.id = 'canvas';
    // Loaded here, not at the top, so that this module imports without the game's WASM.
    const { Sm64VisualEngine } = await import('../scripts/sm64/Sm64Game.js');
    this.engine = await Sm64VisualEngine.create(canvas);
    this.canvas = canvas;
    this.frozen = document.createElement('canvas');
    this.frozen.className = 'freeze';
    this.frozen.hidden = true;
    container.appendChild(this.frozen);
    this.resolution = new ResolutionControl(container, loadResolution(), () => this.showFrozen());
    this.resolution.start();
    this.setAudio(false);
    this.state = this.engine.get_game_state();
    const s0 = this.saveState(); // the start, if it can be saved (the trail's first state)
    if (s0) this.trail.add(0, s0);
    if (!this.headlessFailed && !this.engine.portable_supported) {
      console.info('replay: portable states are unavailable here, so the headless catch-up is off');
      this.headlessFailed = true;
    }
    if (!this.headlessFailed && typeof this.engine.save_state === 'function') {
      // The engine as it is at power-on: each landing starts from it (see landHeadless).
      try {
        this.pristine = this.engine.save_state();
      } catch {
        this.pristine = null;
      }
    }
    if (!this.headlessFailed && this.workerTrail && this.pads.length >= HEADLESS_FILL_MIN) this.ensureHeadless();
    this.raf = requestAnimationFrame(this.tick);
    this.emit();
  }

  setResolution(res) {
    this.resolution?.set(res);
  }

  dispose() {
    this.dead = true;
    this.resolution?.stop();
    cancelAnimationFrame(this.raf);
    this.raf = 0;
    this.setAudio(false);
    this.headless?.terminate();
    this.headless = null;
  }

  setAudio(on) {
    if (this.engine && this.audio !== on) {
      this.engine.set_audio_enabled(on ? 1 : 0);
      this.audio = on;
    }
  }

  get ended() {
    return this.frame >= this.pads.length;
  }

  get seeking() {
    return this.seekTarget !== null;
  }

  play() {
    if (this.ended) return;
    this.playing = true;
    this.lastTime = null;
    this.debt = 0;
    this.dirty = true;
  }

  pause() {
    this.playing = false;
    this.dirty = true;
  }

  toggle() {
    this.playing ? this.pause() : this.play();
  }

  setSpeed(s) {
    this.speed = s;
    this.debt = 0;
    this.dirty = true;
  }

  /**
   * Resizing the canvas clears it, and the game only draws when it steps a
   * frame, so a paused picture would go black. A copy of the last frame is
   * kept (it has to be taken in the same task as the render: the browser
   * clears a WebGL canvas once it has shown it) and shown until the next
   * frame is drawn.
   */
  capture() {
    const f = this.frozen;
    if (!f || !this.canvas || !this.canvas.width) return;
    f.width = this.canvas.width;
    f.height = this.canvas.height;
    f.getContext('2d').drawImage(this.canvas, 0, 0);
    this.hasFrozen = true;
  }

  showFrozen() {
    if (this.hasFrozen && !this.playing && !this.seeking) this.frozen.hidden = false;
  }

  /** Advances one frame while paused. */
  step() {
    if (!this.engine || this.seeking || this.ended) return;
    this.pause();
    this.advance();
    this.frozen.hidden = true;
    this.capture();
    this.rendered = false;
  }

  /**
   * Goes to `target`: forwards by fast-forwarding; backwards by loading the
   * nearest earlier state of the trail and fast-forwarding from there. Returns
   * false if there is none (the emulator has to start over).
   */
  seek(target) {
    target = Math.max(0, Math.min(this.pads.length, Math.round(target)));
    this.headlessToken += 1; // a catch-up under way is replaced by this seek
    if (this.headlessBusy) {
      this.headless?.stop();
      this.headlessBusy = false;
      this.headlessProgress = null;
    }
    const plan = this.planHeadless(target);
    if (plan) {
      this.headlessSeek(target, plan, this.headlessToken);
      return true;
    }
    return this.legacySeek(target);
  }

  /** The seek without the worker: from the nearest state, stepping the rendering build. */
  legacySeek(target) {
    // Backwards, or forwards past a state ahead of here: start from the nearest state.
    const ahead = this.trail.nearest(target - 1);
    if (target < this.frame || (ahead && ahead.frame > this.frame)) {
      if (!this.rewindTo(target) && target < this.frame) return false;
    }
    this.seekTarget = target === this.frame ? null : target;
    this.dirty = true;
    if (this.seekTarget !== null) {
      this.emit(); // the playhead jumps to the target at once
      this.runSeek();
    }
    return true;
  }

  // --- the headless catch-up ---
  //
  // Stepping the rendering build to a far frame costs its full frame time
  // even with the drawing muted. The headless build (a Web Worker, no canvas,
  // GL or audio) steps the same pads several times faster, off the main
  // thread, and its state at the target frame is loaded into the rendering
  // engine as a portable state (docs/portable-state.md), which then plays the
  // last frame(s) itself. The worker fills the trail's gaps afterwards
  // (a pass of its own) and hands back the events on the way. Short seeks keep using the trail; if the worker or
  // the translation fails, the seek is done the old way (and the console says so).

  /** The last frame the worker may take the game to (frames after it are played here, e.g. through a perturbation). */
  headlessLimit() {
    return this.pads.length;
  }

  /** Starts the worker (once). Resolves with whether it came up. */
  ensureHeadless() {
    if (this.headlessFailed) return Promise.resolve(false);
    if (!this.headless) {
      this.headless = new HeadlessClient(this.pads, this.options.makeHeadlessWorker);
      this.headless.start().then(
        () => {
          this.headlessReady = true;
          this.dirty = true;
        },
        (e) => this.failHeadless(e),
      );
    }
    return this.headless.ready.then(() => true, () => false);
  }

  failHeadless(e) {
    if (this.headlessFailed) return;
    this.headlessFailed = true;
    this.headlessReady = false;
    console.warn('replay: the headless catch-up is unavailable, stepping the rendering build instead:', e);
    this.headless?.terminate();
    this.headless = null;
  }

  /**
   * Whether (and how) a seek to `target` goes through the worker: it lands the
   * game at frame `wt` (the frame before the target, so that the target is drawn
   * by the engine itself; at most headlessLimit()), when that saves more than
   * `headlessThreshold` frames over the nearest state of the trail or the
   * game's own position. Returns {wt, maxStart} or null.
   */
  planHeadless(target) {
    if (this.headlessFailed || !this.engine?.portable_supported) return null;
    const wt = Math.min(target - 1, this.headlessLimit());
    if (wt <= 0 || target === this.frame) return null;
    // The worker must start at or before this for the events to stay complete.
    const maxStart = this.trackEvents ? this.evThrough : Infinity;
    let best = this.trail.nearest(target - 1)?.frame ?? -1;
    if (target >= this.frame && this.frame <= target - 1) best = Math.max(best, this.frame);
    if (wt - best <= this.options.headlessThreshold) return null;
    return { wt, maxStart };
  }

  /** Adds states the worker took to the trail (when this player's trail takes portable states). */
  addWorkerStates(states) {
    if (!this.workerTrail || !states?.length) return;
    for (const [f, st] of states) if (!this.trail.has(f)) this.trail.add(f, st);
  }

  async headlessSeek(target, plan, token) {
    const t0 = performance.now();
    const from = this.frame;
    this.seekTarget = target === this.frame ? null : target;
    this.headlessBusy = true;
    this.setAudio(false);
    this.dirty = true;
    this.emit(); // the playhead jumps to the target at once
    const stale = () => token !== this.headlessToken || this.dead;
    let res;
    let landing = false;
    try {
      if (!(await this.ensureHeadless())) throw new Error('the headless worker did not start');
      res = await this.headless.run(
        {
          type: 'goto',
          target: plan.wt,
          maxStart: plan.maxStart,
          every: 0, // taking states on the way costs about as much as the stepping: the fill pass (below) takes them
          land: true,
          events: this.trackEvents,
        },
        (m) => {
          if (stale()) return;
          this.headlessProgress = m.frame;
          this.addWorkerStates(m.states);
          this.dirty = true;
          this.emit();
        },
      );
      if (stale() || res.cancelled) return; // a newer seek took over
      const t1 = performance.now();
      this.addWorkerStates(res.states);
      landing = true;
      this.landHeadless(res);
      landing = false;
      this.lastHeadless = {
        from,
        to: target,
        landed: res.frame,
        start: res.start,
        steps: res.steps,
        workerMs: res.ms,
        landMs: performance.now() - t1,
        totalMs: performance.now() - t0,
        game: res.game,
      };
    } catch (e) {
      if (stale()) return;
      this.headlessErrors += 1;
      console.warn('replay: the headless catch-up failed, stepping the rendering build instead:', e);
      if (this.headlessErrors >= 3 || !this.headlessReady) this.failHeadless(e);
      this.headlessBusy = false;
      this.headlessProgress = null;
      if (landing) this.rewindTo(target); // the engine may be half loaded: a state of the trail first
      if (!this.legacySeek(target)) this.seekTarget = null;
      this.emit();
      return;
    }
    this.headlessBusy = false;
    this.headlessProgress = null;
    if (this.workerTrail && !this.fillRunning) this.runFill(); // the trail's gaps, in the background
    if (this.frame >= target) {
      this.seekTarget = null;
      this.lastTime = null;
      this.debt = 0;
      this.dirty = true;
      this.emit();
    } else {
      this.runSeek(); // the frames after the landing are played here
    }
  }

  /** Loads the worker's state into the rendering engine, as if the game had been played to it. */
  landHeadless(res) {
    // A portable state from the other build loads into the engine as it is at power-on
    // (what the translation was validated against), not over a game that has been loaded
    // and played: that can corrupt the heap.
    if (this.pristine) this.engine.load_state(this.pristine);
    this.engine.load_portable_state(res.state);
    this.frame = res.frame;
    this.state = this.engine.get_game_state();
    if (this.trackEvents) {
      this.events = this.events.filter((e) => e.frame <= res.start).concat(res.events);
      this.evThrough = res.frame;
    } else {
      this.events = this.events.filter((e) => e.frame <= res.frame);
      this.evThrough = Math.min(this.evThrough, res.frame);
    }
    this.rendered = false;
    this.dirty = true;
    if (this.workerTrail && !this.trail.has(res.frame)) this.trail.add(res.frame, res.state);
  }

  /** One pass of the worker over the replay, filling the trail with its states. */
  async runFill() {
    const { every } = this.trail;
    let f = every;
    while (f < this.pads.length && (this.trail.has(f) || this.trailSkipped.has(f))) f += every;
    if (f >= this.pads.length) {
      this.fillDone = true;
      return;
    }
    this.fillRunning = true;
    try {
      const res = await this.headless.run(
        { type: 'fill', target: this.pads.length, every, skip: [...this.trail.states.keys()], land: false, events: false },
        (m) => this.addWorkerStates(m.states),
      );
      if (res.cancelled) return;
      this.addWorkerStates(res.states);
      // Whatever the worker could not take a state at (the intro) is not tried again.
      for (let g = every; g < this.pads.length; g += every) if (!this.trail.has(g)) this.trailSkipped.add(g);
      this.fillDone = true;
    } catch (e) {
      console.warn('replay: the headless trail fill failed:', e);
      this.fillDone = true;
    } finally {
      this.fillRunning = false;
    }
  }

  /** Fast-forwards to the seek target without drawing, in slices that yield to the page. */
  async runSeek() {
    if (this.seekRunning) return; // the running one follows the new target
    this.seekRunning = true;
    this.setAudio(false);
    let unmute = null;
    try {
      while (this.seekTarget !== null && !this.dead && !this.exporting) {
        const t0 = performance.now();
        while (this.frame < this.seekTarget && performance.now() - t0 < SEEK_SLICE_MS) {
          // Everything but the last frame is quiet; the last one is drawn.
          this.quiet = this.frame < this.seekTarget - 1;
          if (this.quiet && !unmute) unmute = muteDrawing(this.canvas);
          else if (!this.quiet && unmute) { unmute(); unmute = null; }
          this.advance();
        }
        this.quiet = false;
        if (this.frame >= this.seekTarget) {
          this.seekTarget = null;
          this.lastTime = null;
          this.debt = 0;
          this.state = this.engine.get_game_state();
          break;
        }
        this.dirty = true;
        this.emit();
        await yieldTask();
      }
    } finally {
      this.quiet = false;
      if (unmute) unmute();
      this.seekRunning = false;
      this.dirty = true;
      this.emit();
    }
  }

  /**
   * One step of the background pass that fills the trail: the first missing
   * state, `every` frames on from the one before it, taken without drawing,
   * after which the game is put back exactly where it was. Cheap enough to
   * run between animation frames of normal play; a seek or export is never
   * waited for (it just starts after the hop).
   */
  hopTrail() {
    const { every } = this.trail;
    let f = every;
    while (f < this.pads.length && (this.trail.has(f) || this.trailSkipped.has(f))) f += every;
    if (f >= this.pads.length) return false; // complete
    const base = this.trail.nearest(f - 1);
    if (!base || f - base.frame > every * 4) { this.trailSkipped.add(f); return true; }
    const cur = this.saveState();
    if (!cur) return true; // can't put the game back here: try later
    const keep = { frame: this.frame, state: this.state, events: this.events.slice(), evThrough: this.evThrough };
    const unmute = muteDrawing(this.canvas);
    this.hopping = true;
    try {
      this.loadState(base.state, base.frame);
      this.quiet = true;
      while (this.frame < f) this.advance();
      this.quiet = false;
      this.checkpoint(); // takes the state at f
      if (!this.trail.has(f)) this.trailSkipped.add(f);
    } finally {
      this.quiet = false;
      this.hopping = false;
      unmute();
      this.loadState(cur, keep.frame);
      this.state = keep.state;
      this.events = keep.events;
      this.evThrough = keep.evThrough;
      this.rendered = false;
    }
    return true;
  }

  // --- the trail of save states ---

  /**
   * Takes a state of the game as it is (before frame `this.frame`), or null if
   * it can't. A portable state (`export_portable_state`: small, and loads in
   * any build) where the engine has them; else the engine's own raw snapshot
   * as a delta against the first one (`save_state_delta`). Subclasses may use
   * other states (and override loadState to match).
   */
  saveState() {
    const e = this.engine;
    if (!e) return null;
    try {
      if (e.portable_supported) return e.export_portable_state();
      if (typeof e.save_state_delta !== 'function') return null;
      if (!this.rawBase) {
        this.rawBase = e.save_state();
        return { raw: this.rawBase, base: null };
      }
      return { raw: e.save_state_delta(this.rawBase), base: this.rawBase };
    } catch {
      return null; // no state here (the intro's Mario head runs): try again later
    }
  }

  /** Makes the game the one `state` (from saveState) was taken of, at movie frame `frame`. */
  loadState(state, frame) {
    if (state?.raw) this.engine.load_state(state.raw, state.base);
    else this.engine.load_portable_state(state);
    this.frame = frame;
    this.state = this.engine.get_game_state();
    this.events = this.events.filter((e) => e.frame <= frame);
    this.evThrough = Math.min(this.evThrough, frame);
    this.rendered = false;
    this.dirty = true;
  }

  /** Called before every frame: keeps a state when one is due. */
  checkpoint() {
    if (!this.trail.due(this.frame)) return;
    const s = this.saveState();
    if (s) this.trail.add(this.frame, s);
  }

  /** Loads the latest state before `target` (so at least one frame is drawn after it); false if there is none. */
  rewindTo(target) {
    const c = this.trail.nearest(Math.max(0, target - 1));
    if (!c) return false;
    this.loadState(c.state, c.frame);
    return true;
  }

  /** Plays one frame in the engine (a subclass may route the pad elsewhere, e.g. through a perturbation). */
  stepGame(pad) {
    this.engine.step_game(pad);
  }

  advance() {
    this.checkpoint();
    const pad = this.pads[this.frame];
    this.stepGame(pad);
    this.rendered = true;
    this.frame += 1;
    if (this.quiet && !this.trackEvents) { this.dirty = true; return; }
    const prev = this.state;
    const s = this.engine.get_game_state();
    this.state = s;
    if (this.evThrough === this.frame - 1) this.evThrough = this.frame;
    if (prev) {
      if (s.numStars > prev.numStars) {
        this.events.push({ frame: this.frame, kind: 'star', label: `Star ${s.numStars}` });
      }
      // Area 0 = nothing loaded yet (boot, title screen).
      if (prev.areaIndex !== 0 && (s.courseNum !== prev.courseNum || s.areaIndex !== prev.areaIndex)) {
        this.events.push({ frame: this.frame, kind: 'area', course: s.courseNum, area: s.areaIndex });
      }
    }
    this.dirty = true;
  }

  tick = (time) => {
    if (!this.engine) return;
    if (this.exporting) {
      this.raf = requestAnimationFrame(this.tick);
      return;
    }
    const start = performance.now();
    if (this.seeking) {
      this.setAudio(false); // runSeek() does the work
    } else if (this.playing) {
      this.setAudio(this.speed === 1);
      if (this.speed === Infinity) {
        while (!this.ended && performance.now() - start < BUDGET_MS) this.advance();
      } else {
        if (this.lastTime !== null) {
          // Frames owed since the last animation frame; resync after a stall.
          this.debt += ((time - this.lastTime) / 1000) * FPS * this.speed;
          if (this.debt > 10 * Math.max(1, this.speed)) this.debt = 1;
        }
        this.lastTime = time;
        while (this.debt >= 1 && !this.ended && performance.now() - start < BUDGET_MS) {
          this.advance();
          this.debt -= 1;
        }
      }
      if (this.ended) this.playing = false;
    } else {
      this.setAudio(false);
    }
    if (this.rendered) {
      this.rendered = false;
      this.frozen.hidden = true;
      if (!this.seeking && this.speed !== Infinity) this.capture();
    }
    // Background: fill the trail (paused: at once; playing: now and then).
    if (!this.seeking && !this.exporting && !this.dead && time - this.lastHop > (this.playing ? HOP_GAP_MS : 0)) {
      this.lastHop = time;
      if (this.workerTrail && !this.headlessFailed && this.options.headless) {
        // The worker fills the trail (off this thread); until it is up, the hops do.
        if (this.headlessReady) {
          if (!this.fillRunning && !this.fillDone) this.runFill();
        } else if (!this.headless) this.hopTrail();
        else if (this.pads.length < HEADLESS_FILL_MIN) this.hopTrail();
      } else this.hopTrail();
    }
    if (this.dirty) {
      this.dirty = false;
      this.emit();
    }
    this.raf = requestAnimationFrame(this.tick);
  };

  emit() {
    this.onChange?.({
      frame: this.headlessProgress ?? this.frame,
      total: this.pads.length,
      playing: this.playing,
      speed: this.speed,
      seeking: this.seeking,
      seekTarget: this.seekTarget,
      state: this.state,
      pad: this.frame > 0 ? this.pads[this.frame - 1] : null,
      events: this.events.slice(),
    });
  }
}
