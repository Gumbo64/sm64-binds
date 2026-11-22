# Changelog

All three packages (Rust `sm64-binds`, Python `sm64-binds`, JavaScript
`sm64-binds-frontend`) are released together at the same version.

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
