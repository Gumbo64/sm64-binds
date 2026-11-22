// Drives the emulator: real-time playback at any speed, single-frame steps,
// fast-forward seeking and event detection. Backwards seeking loads the
// nearest earlier state of a Trail of engine save states; if there is none
// (the intro, where no state can be taken) seek() returns false and the app
// starts a fresh emulator (typically by reloading).

import { FPS } from './m64.js';
import { ResolutionControl, loadResolution } from './resolution.js';
import { Trail } from './trail.js';

export const SPEEDS = [0.25, 0.5, 1, 2, 4, 8, 16, Infinity];

// Wall-clock budget per animation frame while fast-forwarding / playing at
// max speed, so the page stays responsive.
const BUDGET_MS = 14;
// A seek works in slices this long between yields to the page (nothing is
// drawn or heard while it runs, so a long slice is fine).
const SEEK_SLICE_MS = 40;
// While playing, the background trail pass gets one hop this often.
const HOP_GAP_MS = 700;

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
  constructor(pads, onChange) {
    this.pads = pads;
    this.onChange = onChange;
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
    const keep = { frame: this.frame, state: this.state, events: this.events.slice() };
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
      this.hopTrail();
    }
    if (this.dirty) {
      this.dirty = false;
      this.emit();
    }
    this.raf = requestAnimationFrame(this.tick);
  };

  emit() {
    this.onChange?.({
      frame: this.frame,
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
