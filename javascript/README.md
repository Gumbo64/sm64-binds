# sm64-binds-frontend

Mario 64 binds with graphics for the browser. [sm64-binds](https://github.com/Gumbo64/sm64-binds)

To avoid shipping copyrighted ROM data, the WASM module is distributed XOR'd
against a US `.z64` ROM and recombined in the browser at runtime. The ROM is
required: a US `z64` copy, exactly 8.00 MB.

## Install

```
npm install sm64-binds-frontend
```

## Usage

Drop in the `<FileUpload>` component, let the user pick their ROM, then drive
the game:

```jsx
import { useRef } from "react";
import { FileUpload, sm64_record, GameConfig, GamepadButtons } from "sm64-binds-frontend";

function Game({ hasRom }) {
    const canvas = useRef(null);

    if (!hasRom) {
        return <FileUpload setHasRom={...} />;
    }

    return <canvas ref={canvas} className="emscripten" />;
}
```

The WASM module draws to a `<canvas>`; it needs the `emscripten` class for
correct sizing.

### Recording and replaying

`sm64_record` plays the game and records every frame's controller input, so a
run can be replayed or solved later:

```js
import { sm64_record, sm64_evaluate, GameConfig } from "sm64-binds-frontend";

const game_config = GameConfig.defaultConfig();
const solution = await sm64_record(canvas, 22, game_config);
const success = await sm64_evaluate(canvas, solution, 22);
```

### Controller input

`GamepadButtons` has the N64 masks (`A_BUTTON`, `START_BUTTON`, `U_JPAD`, and so on).

```js
const pad = engine.get_controller_pad();
if (pad.is_pressed(GamepadButtons.U_JPAD)) {
    // ...
}
```

## API

| Export | Purpose |
| --- | --- |
| `FileUpload` | React component that validates and stores a ROM in IndexedDB. |
| `sm64_record` | Plays and records inputs. |
| `sm64_evaluate` | Replays a recorded solution. |
| `sm64_playback` | Replays with manual control. |
| `GameConfig` | Playback tuning. |
| `GamePad`, `GameState`, `GamepadButtons` | Controller input and game state. |

## Example

See `js_example/` in the [repository](https://github.com/Gumbo64/sm64-binds)
for a complete Vite + React app.

## Licence

ISC. The game itself is © Nintendo; this package contains no ROM or game data.
### Save states

`Sm64VisualEngine` (all synchronous after `create`) has two kinds of state; see
`docs/portable-state.md` for the formats and limits.

| call | what |
|---|---|
| `save_state()` -> `Uint8Array` | raw snapshot of this build's game memory (13 MB), fast; same session / same build only |
| `save_state_delta(base)` -> `Uint8Array` | only the 4 KiB pages that differ from `base` (a full `save_state()`), typically a few hundred KB: for a trail of snapshots. Falls back to a full snapshot if the heap grew |
| `load_state(state, base?)` | loads either (a delta needs its `base`); resets the renderer's texture cache so GL textures match the restored game |
| `export_portable_state()` -> `Uint8Array` | build-neutral state (~0.6 MB) the headless build (Rust `SM64Game::load_portable`, the nodes) loads too; throws while the intro's Mario head runs |
| `load_portable_state(bytes)` | loads a portable state made by either build |
| `portable_supported` | whether the translator WASM loaded and matches this game build |

Seeking with a trail: snapshot every N frames (`save_state_delta` against a
full snapshot kept every few hundred frames, or `export_portable_state` for
storable ones), restore the nearest earlier one and step forward. Audio is not
part of a state (call `set_audio_enabled(0)` while seeking). Code:
`src/scripts/sm64/portable.js` (translator), `Sm64Game.js` (engine methods).

## Replay player (`sm64-binds-frontend/replay`)

A React replay player for any movie of controller pads (`{ button, stick_x, stick_y }`
frames): real-time play at 0.25x to max speed, frame steps, seeking both ways (a
`Trail` of engine save states: portable states, or `save_state_delta` snapshots where
the translator is unavailable, filled by a background pass), a timeline with annotated
regions and star/area events, HUD, controller display, render-resolution options and
MP4 export (WebCodecs + `mp4-muxer`, loaded when the export dialog first opens).

```jsx
import { Viewer, parseReplay } from 'sm64-binds-frontend/replay';
import 'sm64-binds-frontend/replay.css';   // scoped under .sm64-replay

const { pads } = parseReplay(await (await fetch('run.m64')).arrayBuffer());
<Viewer replay={{ pads, regions: [] }} onClose={() => {}} />
```

React and ReactDOM are peer dependencies (bring your own, one copy; with a bundler's
`dedupe` when the package is linked). The game ROM must already be loaded
(`FileUpload`, `getROM`) before a `Viewer` mounts. Loading the replay file, its
annotations and the ROM is the app's job; nothing here fetches.

### Headless catch-up

Fast-forwarding a long replay by stepping the rendering build (even with the draws
skipped) costs its full frame time on the main thread. Since 2.3.0 the `Player`
catches up with the **headless** build of the game instead, in a Web Worker (no canvas,
GL or audio; the same WASM the Rust and Python packages run, a few times faster), and
loads the result into the rendering engine as a portable state:

- *Initial catch-up* (`Viewer` opening at `initial.frame`) and *long seeks* (those that
  would save more than 600 frames of stepping over the nearest trail state or the current
  position, in either direction) go through the worker: it steps the pads to the frame
  before the target, posts a portable state (a transferable `ArrayBuffer`), the engine
  loads it and plays the last frame itself, so the target frame is drawn as usual.
  Short seeks keep using the trail.
- The worker also fills the **trail** in the background (and as a by-product of catch-ups)
  and returns the star/area **events** on the way, so the timeline is complete after a jump.
- It is lazy: the worker, its ~6 MB payload (`sm64_headless.us.wasm.zip.xor.png`, XORed with
  the ROM like the rendering build's) and its decode only happen when a replay of 1500+
  frames is attached or a long seek needs it; the decoded game is cached in IndexedDB next
  to the rendering one (and dropped when `storeROM` stores another ROM).
- If the worker, the headless WASM or the state translation fails, the seek is done the old
  way and the console says `replay: the headless catch-up ...`.

`new Player(pads, onChange, options)` / `<Viewer playerOptions={...}>`: `headless`
(default `true`), `headlessThreshold` (600), `makeHeadlessWorker` (default
`() => new Worker(new URL('./headless.worker.js', import.meta.url), { type: 'module' })`;
pass your own if your bundler does not understand that). Subclasses that keep other
kinds of states or that play some frames differently (SM64Coin's perturbed block):
set `this.workerTrail = false` (the worker's portable states then stay out of the trail)
and override `headlessLimit()` to return the last frame the worker may reach: it replays
the movie's pads as they are, so frames after the limit are played by the engine. After a
catch-up `player.lastHeadless` says what it did (`steps`, `workerMs`, `totalMs`, and the
game's position and action at the landing, for checking against the engine).

The worker's game is always the continuous run of the movie from power-on (exact), and
a landing is the only translation: the engine first returns to its power-on snapshot, then loads
the portable state. Portable states are validated for a few hundred frames after a checkpoint
(`docs/portable-state.md`); over longer stretches a landed game can drift from the continuous
run (the 120-star TAS landed at frame 3000 drifts around frame 23000), as the trail's states can.

Lower level: `Sm64HeadlessEngine` (`src/scripts/sm64/Sm64Headless.js`: the headless build
in JavaScript, in a worker or in Node; `step_game`, `export_portable_state`,
`load_portable_state`, `restart`), `HeadlessClient` (the page's side of the worker).

| Export | Purpose |
| --- | --- |
| `Viewer` | The whole UI. Props: `replay` (`{ pads, regions?, name?, ... }`), `initial` (`{ frame, speed, playing }`), `onRewind(frame, speed, playing)` (called when a backwards seek has no state to load: the app restarts the emulator), `onClose`, `backLabel`, `playerOptions` (`Player`'s third argument), `makePlayer(replay, onChange)` (return a `Player` subclass), `topbarExtra(view)`, `panelExtra(view)`, `regionLink(region)`, `milestones` (sorted frames for the previous/next buttons). |
| `Player` | Drives the engine: `attach(container)`, `play/pause/toggle`, `setSpeed`, `step`, `seek(frame)` (false if it cannot), `dispose`. Calls `onChange(view)` with `frame`, `total`, `playing`, `speed`, `seeking`, `state`, `pad`, `events`. Override `saveState()` / `loadState(state, frame)` / `stepGame(pad)` to play differently. |
| `Trail` | Save states by frame (`add`, `nearest`, `due`), thinned when full. |
| `SPEEDS`, `FPS`, `formatTime` | Playback constants. |
| `parseReplay`, `hasMupenHeader` | Raw 4-bytes-per-frame or Mupen `.m64` bytes to `{ pads, header, trailing }`. |
| `normalizeRegions`, `regionAt` | Region lists `{ from, to?, label?, note?, color? }`. |
| `RESOLUTIONS`, `loadResolution`, `saveResolution` | Render size options (persisted in localStorage). |
| `BUTTONS`, `courseName` | Display constants. |
