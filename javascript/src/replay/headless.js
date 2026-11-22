// The page's side of the headless catch-up: a Web Worker that runs the
// headless build of the game (headless.worker.js) and, on request, steps the
// replay's pads to a frame and hands back the game there as a portable state
// (and, on the way, states for the Player's trail and the star/area events).
// The worker and its ~6 MB game are only created when a Player first wants
// them (see Player's `headless` option).

let nextId = 1;

/** Whether a worker can be made: the browser has Workers, or the caller brings its own `makeWorker`. */
export function headlessSupported(makeWorker) {
  return typeof makeWorker === 'function' || typeof Worker !== 'undefined';
}

/** `pads` as the flat Int32Array the worker steps: [button, stick_x, stick_y, ...]. */
export function flattenPads(pads) {
  const out = new Int32Array(pads.length * 3);
  for (let i = 0; i < pads.length; i++) {
    out[3 * i] = pads[i].button;
    out[3 * i + 1] = pads[i].stick_x;
    out[3 * i + 2] = pads[i].stick_y;
  }
  return out;
}

export class HeadlessClient {
  /**
   * `makeWorker`: how to create the worker; the default is what bundlers
   * (Vite, webpack 5) understand. Pass your own if yours does not.
   */
  constructor(pads, makeWorker) {
    this.pads = pads;
    this.makeWorker =
      makeWorker ?? (() => new Worker(new URL('./headless.worker.js', import.meta.url), { type: 'module' }));
    this.worker = null;
    this.ready = null; // Promise, resolved with the worker's build id
    this.jobs = new Map(); // id -> {resolve, reject, onProgress}
    this.dead = false;
  }

  /** Creates the worker (once); resolves when its game is up. Rejects if it can't be. */
  start() {
    this.ready ??= new Promise((resolve, reject) => {
      let w;
      try {
        w = this.worker = this.makeWorker();
      } catch (e) {
        reject(e);
        return;
      }
      w.onmessage = (e) => this._message(e.data, resolve, reject);
      w.onerror = (e) => {
        const err = new Error(e.message || 'the headless worker failed');
        reject(err);
        this._failAll(err);
      };
      w.postMessage({ type: 'init', pads: flattenPads(this.pads) });
    });
    return this.ready;
  }

  _message(m, resolveReady, rejectReady) {
    if (m.type === 'ready') return resolveReady(m.codeId);
    if (m.type === 'log') return console.warn(m.message);
    const job = this.jobs.get(m.id);
    if (m.type === 'error') {
      if (!this.jobs.size && !job) return rejectReady(new Error(m.message)); // init failed
      if (job) {
        this.jobs.delete(m.id);
        job.reject(new Error(m.message));
      }
    } else if (m.type === 'progress') job?.onProgress?.(m);
    else if (m.type === 'done' && job) {
      this.jobs.delete(m.id);
      job.resolve(m);
    }
  }

  _failAll(err) {
    for (const j of this.jobs.values()) j.reject(err);
    this.jobs.clear();
  }

  /**
   * Runs a job: {type: 'goto'|'fill', target, maxStart, every, skip,
   * land, events}. A newer job replaces an older one,
   * which resolves with {cancelled: true}. `onProgress` gets {frame, states}.
   */
  async run(job, onProgress) {
    await this.start();
    if (this.dead) return { cancelled: true };
    for (const [id, j] of this.jobs) {
      this.jobs.delete(id);
      j.resolve({ cancelled: true });
    }
    const id = nextId++;
    return new Promise((resolve, reject) => {
      this.jobs.set(id, { resolve, reject, onProgress });
      const transfer = [];
      this.worker.postMessage({ maxStart: Infinity, ...job, id }, transfer);
    });
  }

  /** Cancels the running job, if any. */
  stop() {
    for (const [id, j] of this.jobs) {
      this.jobs.delete(id);
      j.resolve({ cancelled: true });
    }
    this.worker?.postMessage({ type: 'stop' });
  }

  terminate() {
    this.dead = true;
    this.stop();
    this.worker?.terminate();
    this.worker = null;
  }
}
