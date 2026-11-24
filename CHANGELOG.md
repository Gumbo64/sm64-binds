# Changelog

All three packages (Rust `sm64-binds`, Python `sm64-binds`, JavaScript
`sm64-binds-frontend`) are released together at the same version.

## 2.3.1

- **Fixed:** loading a portable state across builds (headless → rendering) now renders and plays
  like the original: no more black screen, missing/doubled Mario, corrupted object values, or crash
  after a level change. Checked at 167 checkpoints and by a differential fuzz.
- **Changed:** portable format v2 (copies memory that stale pointers still read; v1 still loads).
- **Added:** `portable_check quick` and `portable_fuzz` for checking translations.
- The game WASM is unchanged.

## 2.3.0

### Added

- **Headless catch-up for the replay player.** The `Player` fast-forwards (the viewer opening
  a replay at a frame, long seeks in either direction) with the headless build of the game in
  a Web Worker and loads the resulting portable state into the rendering engine, instead of
  stepping the rendering build with its draws muted. The worker also fills the trail and
  collects the star/area events. Measured on the 120-star TAS (Chromium, software GL, cold page
  including the worker and the headless game's decode): frame 10k in 0.6 s instead of 3.1 s, 50k in
  1.5 s instead of 13.9 s, 100k in 2.6 s instead of 24 s; the landed game equals the stepped one
  (position, action, stars, events) at every frame checked. Short seeks keep using the trail; any failure falls back to the old
  way with a console message. New: `Sm64HeadlessEngine` (the headless game in JavaScript: a
  worker or Node), `HeadlessClient`, `headless.worker.js`, the headless payload
  `sm64_headless.us.wasm.zip.xor.png` (lazy: only the worker fetches it, cached in IndexedDB),
  `Player` options (`headless`, `headlessThreshold`, `makeHeadlessWorker`), `Viewer`'s
  `playerOptions`, and the `headlessLimit()` / `workerTrail` hooks for subclasses.
- `wasmCodeId` moved to `portable.js` (still re-exported from `Sm64Game.js`); `storeROM` also
  drops the cached headless WASM.
- Known limit, not new: a portable state is only checked for a few hundred frames after it. In
  the 120-star TAS, a game landed at frame 3000 (by the worker, or by the trail's states) drifts
  from the continuous run around frame 23000 (`gBodyStates` is the first variable to differ);
  landings at 1000, 70000 and 100000 stay exact for 30000 frames. The worker itself is always the
  continuous run from power-on, so landing is the only translation involved.
- The game WASM itself is unchanged (headless sha1 `a6a197b5ec48fa11725d009eed4af37fbacd00c0`).

## 2.2.0

### Added

- **Replay player** for the JavaScript package, as a subpath export: `sm64-binds-frontend/replay`
  (React `Viewer`, `Player`, `Trail`, `parseReplay`, `normalizeRegions`, ...) and
  `sm64-binds-frontend/replay.css`. Real-time play, speeds, frame steps, seeking both ways
  through a trail of engine save states (portable states, or `save_state_delta` snapshots when
  the translator is unavailable), timeline with regions and events, HUD, input display,
  resolution options, MP4 export. Moved here from sm64-viewer, which (and SM64Coin's web app)
  now import it from this package. See the README's "Replay player" section.
- `react` and `react-dom` are now peer dependencies of the JavaScript package (they were
  dependencies); `mp4-muxer` is a new dependency (loaded only by the export dialog).

## 2.1.1

- Release fix: Cargo.lock pins wasm-bindgen 0.2.108 again (0.2.129 broke wasmer 6.1.0's JS backend, so 2.1.0's release build failed and it was never published). Otherwise identical to 2.1.0.

## 2.1.0

**Breaking:** the in-game controller perturbation is gone from the game and the
bindings. Perturbation lives with whoever needs it (SM64Coin runs its own, outside
the game): `RngConfig`, `rng_pad`, `set_rng_config`, `set_rng_seed`, `using_rng` and
`has_rng` are removed from the Rust, Python and JavaScript APIs, with their tests and
benchmarks.

### Added

- **Portable save states**: a state saved by the headless build (Rust, Python, the nodes)
  loads in the rendering build (the browser) and the other way round, and the game goes
  on identically. Rust: `SM64Game::export_portable` / `load_portable`, `sm64_binds::portable::Portable`
  (`to_bytes` / `from_bytes`, format "SM64PORT"). JavaScript: `Sm64VisualEngine.export_portable_state()` /
  `load_portable_state(bytes)`. See `docs/portable-state.md` for the format, what a state holds and
  its limits (no state while the intro's Mario head runs; audio is not transferred).
- `Sm64VisualEngine.save_state()` / `load_state()`: the rendering build's own raw snapshot (13 MB),
  for seeking and undo within a session. Loading resets the renderer's texture cache.
- The translation as a WASM module for the browser (`rust/portable-wasm`, shipped as
  `javascript/src/scripts/sm64/pkg/portable_wasm.js`), and generators for its tables:
  `maketools/gen_portable_tables.py` (from the linker maps, WASMs and DWARF of the two builds) and
  `maketools/portable_wasm.py` (`make portable`).
- `rust/examples/portable_check.rs` and `javascript/e2e/portable_check.mjs`: cross-build validation of
  states with a TAS movie in headless Chromium.

### Changed

- The rendering build writes a linker map (`build/us_web/sm64.map`).
- The Mupen64 state tables and the precompiled LLVM module (`rust/pkg/*.wasmu`) were regenerated for the
  stock game build.

## 2.0.1

- Export the engine and ROM cache helpers from the JavaScript entry.

## 2.0.0

- Fast save states (`sync_from`), tests, benchmarks, the N64 oracle.
