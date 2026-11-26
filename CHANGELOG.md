# Changelog

All three packages (Rust `sm64-binds`, Python `sm64-binds`, JavaScript
`sm64-binds-frontend`) are released together at the same version.

## 2.3.3

**The game build changes** (build id 897378b4... -> 73f3e66d..., the same build as 2.1.2), so a state or replay
from 2.3.2 is not interchangeable with this one where the differences below show. No API change.

- Game fix, port vs N64: `bhv_small_water_wave_loop` (water_objs.inc.c) and `bhv_object_bubble_loop`
  (bubble.inc.c) guarded their splash spawns with `#ifdef USE_SYSTEM_MALLOC` (always defined here) instead of
  `MALLOC_OBJECTS`, like `try_to_spawn_object`. With the object pool full the port spawned a splash (after evicting
  one more unimportant object) that the N64 refuses, and the RNG and object list parted from there. Runs that
  never fill the pool in water are unaffected.
- Game fix, port vs N64: the object and effects memory pools (`gObjectMemoryPool`, 0x800 bytes;
  `gEffectsMemoryPool`, 0x4000) had no limit with `USE_SYSTEM_MALLOC`. They now run the N64's allocator
  (memory.c: first fit over the free list, 8-byte block headers, sizes rounded to 4, the same pool sizes), so every
  allocation succeeds or fails, at the same offset, as on the N64. The Wiggler never frees its 4 chain segments:
  on the N64 its 24th spawn in one level finds no room, and that Wiggler stays hidden and uninitialized (no body,
  no star fight; it retries every frame), where the port kept spawning it. Chain chomps free theirs and are
  unaffected unless the pool is full. The port-only goddard heap (size 0) keeps the unlimited allocator.
  `struct MemoryPool` grows by the N64 fields (offsets instead of pointers, so states need no translation for them).
- Mupen64 states: the object memory pool is copied block for block (free list included), so allocations after a
  load succeed or fail as on the N64; chain segments are placed at the N64's offsets in it.
- The WASM payloads, the precompiled module, the Mupen and portable-state tables (build ids; the `MemoryPool`
  layout) and the browser translator were regenerated for the new build.

## 2.3.2

- **Fixed:** loading a portable state checks the build id it records: a state from another build
  is refused instead of loaded with wrong function pointers. Every state 2.1.0–2.3.1 wrote loads.
- Portable diagnostics: `Walked::bad_nodes` is filled; unresolved pointers in blocks are named
  `block@<address>`. Exported bytes are unchanged.

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
