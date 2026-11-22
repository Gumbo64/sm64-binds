// Render resolution and shape.
//
// The game sizes its framebuffer from the canvas element's on-screen size,
// but only when the window fires a `resize` event, and only while the canvas
// still has id="canvas" (the Emscripten runtime looks it up by that id). So:
//
//  - `fit`   the box is 4:3 and as large as the stage allows (the game's own
//            shape);
//  - `fill`  the box is exactly the stage's shape, rendered at `k` times the
//            stage's pixel size (k < 1 gives chunky pixels). The game widens
//            its view to the new aspect ratio;
//  - `fixed` the box has exactly `w` x `h` CSS pixels, so the game renders at
//            that size, and is then scaled to fit the stage.
//
// Scaled boxes (fill with k != 1, fixed) use a CSS transform. The game
// measures the transformed size, so the transform is lifted while it handles
// a `resize` event and put back right after.

const fixed = (w, h, note = '') => ({ id: `${w}x${h}`, kind: 'fixed', w, h, label: `${w} × ${h}${note}` });

// The first entry is the default.
export const RESOLUTIONS = [
  { id: 'fill', kind: 'fill', k: 1, label: 'Fill window (any shape)' },
  { id: 'fit', kind: 'fit', label: 'Original shape (4:3)' },
  { id: 'fill-0.5', kind: 'fill', k: 0.5, label: 'Fill window, half resolution' },
  { id: 'fill-0.25', kind: 'fill', k: 0.25, label: 'Fill window, quarter resolution' },
  fixed(320, 240, ' (N64)'),
  fixed(640, 480),
  fixed(960, 720),
  fixed(1280, 960),
  fixed(1920, 1440),
  fixed(854, 480, ' (16:9)'),
  fixed(1280, 720, ' (16:9)'),
  fixed(1920, 1080, ' (16:9)'),
];

// (the key predates the shared player; kept so saved choices survive)
const KEY = 'sm64-viewer:resolution';

export function loadResolution() {
  try {
    const id = localStorage.getItem(KEY);
    return RESOLUTIONS.find((r) => r.id === id) ?? RESOLUTIONS[0];
  } catch {
    return RESOLUTIONS[0];
  }
}

export function saveResolution(res) {
  try {
    localStorage.setItem(KEY, res.id);
  } catch {
    // Not persisted; the choice still applies to this session.
  }
}

const BOX = 'flex:none;max-width:none;aspect-ratio:auto;transform:none';

/** Applies a resolution to the canvas box and keeps it fitted to the stage. */
export class ResolutionControl {
  constructor(box, res, beforeResize) {
    this.beforeResize = beforeResize;
    this.box = box;
    this.stage = box.parentElement;
    this.res = res;
    this.fullscreen = false;
    this.syncing = false;
    this.pre = () => {
      if (this.scaled()) this.box.style.transform = 'none';
    };
    this.post = () => {
      if (!this.syncing) this.apply();
    };
    this.observer = new ResizeObserver(() => {
      if (this.effective().kind === 'fixed') this.fit();
      else this.apply();
    });
  }

  /** Start listening. Call after the engine exists, so the game's own
   * `resize` handler is registered first and runs before `post`. */
  start() {
    window.addEventListener('resize', this.pre, true);
    window.addEventListener('resize', this.post);
    this.observer.observe(this.stage);
    this.apply();
  }

  stop() {
    window.removeEventListener('resize', this.pre, true);
    window.removeEventListener('resize', this.post);
    this.observer.disconnect();
  }

  set(res) {
    this.res = res;
    this.apply();
  }

  /** In fullscreen the 4:3 box would leave bars, so it fills the screen. */
  setFullscreen(on) {
    this.fullscreen = on;
    this.apply();
  }

  effective() {
    return this.res.kind === 'fit' && this.fullscreen ? { kind: 'fill', k: 1 } : this.res;
  }

  scaled() {
    const r = this.effective();
    return r.kind === 'fixed' || (r.kind === 'fill' && r.k !== 1);
  }

  /** The box's size in CSS pixels, which is what the game renders at. */
  boxSize() {
    const r = this.effective();
    if (r.kind === 'fixed') return [r.w, r.h];
    if (r.kind === 'fill') {
      return [Math.max(16, Math.floor(this.stage.clientWidth * r.k)), Math.max(16, Math.floor(this.stage.clientHeight * r.k))];
    }
    return null;
  }

  apply() {
    this.beforeResize?.();
    const size = this.boxSize();
    this.box.style.cssText = size ? `width:${size[0]}px;height:${size[1]}px;${BOX}` : '';
    this.syncing = true;
    window.dispatchEvent(new Event('resize'));
    this.syncing = false;
    this.fit();
  }

  fit() {
    const size = this.boxSize();
    if (!size || !this.scaled()) {
      this.box.style.transform = '';
      return;
    }
    const r = this.effective();
    if (r.kind === 'fill') {
      this.box.style.transform = `scale(${this.stage.clientWidth / size[0]}, ${this.stage.clientHeight / size[1]})`;
    } else {
      this.box.style.transform = `scale(${Math.min(this.stage.clientWidth / size[0], this.stage.clientHeight / size[1])})`;
    }
  }
}
