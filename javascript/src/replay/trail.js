// A trail of save states by frame, so that seeking in either direction needs
// only the nearest earlier state plus a fast replay to the target. Generic:
// a state is whatever the owner's save function returns (the viewer's
// portable states; the SM64Coin block page's memory snapshot plus the
// perturbation's state), the trail only keeps them by frame, one every
// `every` frames, thinned when full.

export class Trail {
  constructor(every = 900, max = 400) {
    this.every = every;
    this.max = max;
    this.states = new Map(); // frame -> state
  }

  get size() {
    return this.states.size;
  }

  has(frame) {
    return this.states.has(frame);
  }

  /** Whether the game, at `frame`, is due a state. */
  due(frame) {
    return frame % this.every === 0 && !this.states.has(frame);
  }

  /** Keeps `state` for `frame`; when full, drops every other state (not the first). */
  add(frame, state) {
    if (this.states.size >= this.max) {
      [...this.states.keys()]
        .sort((a, b) => a - b)
        .forEach((f, i) => i % 2 === 1 && this.states.delete(f));
    }
    this.states.set(frame, state);
  }

  /** The latest state at or before `target`: {frame, state}, or null. */
  nearest(target) {
    let best = -1;
    for (const f of this.states.keys()) if (f <= target && f > best) best = f;
    return best < 0 ? null : { frame: best, state: this.states.get(best) };
  }
}
